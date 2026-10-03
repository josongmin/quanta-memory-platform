use super::*;

#[test]
fn sink_panic_payloads_are_retired_once_and_callback_is_quarantined() {
    for panic_on_drop in [false, true] {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let broker = crate::ProcessMemoryBrokerBuilder::new()
            .with_preset(crate::MemoryPreset::Development)
            .with_event_sink(Arc::new(PayloadPanickingSink {
                calls: calls.clone(),
                drops: drops.clone(),
                panic_on_drop,
            }))
            .build()
            .expect("broker");
        for _ in 0..8 {
            let charge = broker
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "hostile-sink".into(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    1,
                )
                .expect("sink cannot alter admission");
            drop(charge);
        }
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
        sep23_assert_fully_drained(&broker);
    }
}
#[test]
fn reclaimer_panic_payloads_are_retired_once_and_admission_remeasures() {
    for panic_on_drop in [false, true] {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let policy = ResidencyPolicy::new(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(Arc::new(PayloadPanickingReclaimer {
                calls: calls.clone(),
                drops: drops.clone(),
                panic_on_drop,
            })),
        )
        .expect("broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "baseline".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100,
            )
            .expect("baseline");
        for _ in 0..8 {
            assert!(matches!(
                broker.reserve(
                    MemoryClass::ArtifactCache,
                    "request".into(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    1,
                ),
                Err(MemoryError::Capacity(_))
            ));
        }
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
        assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 100);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }
}
#[test]
fn event_sink_observation_reentry_delivers_outer_event_once() {
    let sink = Arc::new(RecursiveObserveEventSink {
        broker: Mutex::new(None),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let broker = crate::ProcessMemoryBrokerBuilder::new()
        .with_preset(crate::MemoryPreset::Development)
        .with_event_sink(sink.clone())
        .build()
        .expect("broker");
    *sink.broker.lock().expect("broker slot") = Some(broker.clone());

    broker.record_advisory_event(MemoryEvent::new(
        MemoryOperation::ObserveAllocator,
        MemoryOutcome::Succeeded,
        None,
    ));

    assert_eq!(
        sink.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the nested observe event must be suppressed at the same sink boundary"
    );
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn same_broker_reclaimer_reserve_reentry_is_bounded_and_typed() {
    let reclaimer = Arc::new(RecursiveReserveReclaimer {
        broker: Mutex::new(None),
        calls: std::sync::atomic::AtomicUsize::new(0),
        nested_error: Mutex::new(None),
    });
    let policy = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    *reclaimer.broker.lock().expect("broker slot") = Some(broker.clone());
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "outer".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100,
        )
        .expect("fill hard cap");

    let denied = broker.reserve_managed_charge(
        MemoryClass::ArtifactCache,
        "outer".into(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        1,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(reclaimer.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(matches!(
        reclaimer.nested_error.lock().expect("nested error").as_ref(),
        Some(MemoryError::OperationInProgress(_))
    ));
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 100);
    drop(baseline);
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn concurrent_reclaimer_reentry_does_not_start_a_callback_storm() {
    let reclaimer = Arc::new(ThreadSpawnReclaimer {
        broker: Mutex::new(None),
        calls: std::sync::atomic::AtomicUsize::new(0),
        nested_error: Mutex::new(None),
    });
    let policy = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    *reclaimer.broker.lock().expect("broker slot") = Some(broker.clone());
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "outer".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100,
        )
        .expect("fill hard cap");

    let denied = broker.reserve_managed_charge(
        MemoryClass::ArtifactCache,
        "outer".into(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        1,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(reclaimer.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(matches!(
        reclaimer.nested_error.lock().expect("nested error").as_ref(),
        Some(MemoryError::Capacity(_))
    ));
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 100);
    drop(baseline);
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn independent_outer_reserves_share_one_in_flight_reclaimer() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Duration;

    struct HeldReclaimer {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        calls: AtomicUsize,
    }
    impl MemoryReclaimer for HeldReclaimer {
        fn reclaim(
            &self,
            _request: &ReclaimRequest,
        ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.entered.send(()).expect("parent observes callback entry");
            self.release.lock().expect("release channel").recv().expect("parent releases callback");
            Ok(crate::ports::reclaim::ReclaimOutcome::default())
        }
    }

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let reclaimer = Arc::new(HeldReclaimer {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        calls: AtomicUsize::new(0),
    });
    let policy = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:outer".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100,
        )
        .expect("fill hard cap");

    let first_broker = broker.clone();
    let first = std::thread::spawn(move || {
        first_broker.reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:first".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        )
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).expect("first callback entered");
    let (second_tx, second_rx) = mpsc::channel();
    let second_broker = broker.clone();
    let second = std::thread::spawn(move || {
        let result = second_broker.reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:second".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        );
        second_tx.send(result).expect("parent receives second outcome");
    });
    let second_while_first_held = second_rx.recv_timeout(Duration::from_secs(5));
    release_tx.send(()).expect("release first callback even on failed second outcome");
    let first_result = first.join().expect("first thread");
    second.join().expect("second thread");
    assert!(matches!(second_while_first_held, Ok(Err(MemoryError::Capacity(_)))));
    assert!(matches!(first_result, Err(MemoryError::Capacity(_))));
    assert_eq!(reclaimer.calls.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(broker.ledger_snapshot().expect("unchanged").total_owner_bytes(), 100);
    drop(baseline);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn soft_and_hard_reclaim_plans_share_one_callback_in_both_arrival_orders() {
    use std::sync::mpsc;
    use std::time::Duration;

    struct HeldReclaimer {
        entered: mpsc::Sender<ReclaimTrigger>,
        release: Mutex<mpsc::Receiver<()>>,
        triggers: Mutex<Vec<ReclaimTrigger>>,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl MemoryReclaimer for HeldReclaimer {
        fn reclaim(
            &self,
            request: &ReclaimRequest,
        ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
            self.triggers.lock().expect("triggers").push(request.trigger);
            if self.calls.fetch_add(1, AtomicOrdering::SeqCst) == 0 {
                self.entered.send(request.trigger).expect("first callback entered");
                self.release.lock().expect("release channel").recv().expect("first released");
            }
            Ok(crate::ports::reclaim::ReclaimOutcome::default())
        }
    }

    for soft_first in [true, false] {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reclaimer = Arc::new(HeldReclaimer {
            entered: entered_tx,
            release: Mutex::new(release_rx),
            triggers: Mutex::new(Vec::new()),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let policy = if soft_first {
            ResidencyPolicy::new(1_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 50, 100, false))
        } else {
            ResidencyPolicy::new(1_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false))
                .with_owner_quota("soft", OwnerQuota { soft_cap_bytes: 10, hard_cap_bytes: 100 })
        };
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let baseline_bytes = if soft_first { 40 } else { 80 };
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "baseline".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                baseline_bytes,
            )
            .expect("baseline below soft or hard boundary");
        let first_broker = broker.clone();
        let first = std::thread::spawn(move || {
            first_broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                if soft_first { "soft" } else { "hard" }.into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                if soft_first { 20 } else { 30 },
            )
        });
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(5)).expect("first callback held"),
            if soft_first { ReclaimTrigger::SoftThreshold } else { ReclaimTrigger::CapacityDenial }
        );
        let (second_tx, second_rx) = mpsc::channel();
        let second_broker = broker.clone();
        let second = std::thread::spawn(move || {
            let result = second_broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                if soft_first { "hard" } else { "soft" }.into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                if soft_first { 70 } else { 20 },
            );
            second_tx.send(result).expect("parent receives competing outcome");
        });
        let second_during_first = second_rx.recv_timeout(Duration::from_secs(5));
        release_tx.send(()).expect("release held callback even if peer timed out");
        let first_result = first.join().expect("first request thread");
        second.join().expect("second request thread");
        let second_result = second_during_first.expect("peer must finish while callback held");
        assert_eq!(first_result.is_ok(), soft_first);
        assert_eq!(second_result.is_ok(), !soft_first);
        let denied = if soft_first { &second_result } else { &first_result };
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        let observation = broker.observe().expect("single live postimage");
        assert_eq!(observation.semantic_bytes, if soft_first { 60 } else { 100 });
        assert_eq!(observation.ledger.live_ticket_count, 2);
        assert_eq!(observation.reclaim.attempts, 1);
        assert_eq!(observation.reclaim.terminal_denials, 1);
        assert_eq!(reclaimer.calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(
            reclaimer.triggers.lock().expect("triggers").as_slice(),
            &[if soft_first {
                ReclaimTrigger::SoftThreshold
            } else {
                ReclaimTrigger::CapacityDenial
            }]
        );
        drop(first_result);
        drop(second_result);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }
}
#[test]
fn held_reclaim_remeasures_concurrent_policy_and_slot_changes() {
    use std::sync::mpsc;
    use std::time::Duration;

    struct HeldReclaimer {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl MemoryReclaimer for HeldReclaimer {
        fn reclaim(
            &self,
            _request: &ReclaimRequest,
        ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.entered.send(()).expect("callback entered");
            self.release.lock().expect("release channel").recv().expect("callback released");
            Ok(crate::ports::reclaim::ReclaimOutcome::default())
        }
    }

    for (competing_bytes, should_admit) in [(120, true), (130, false)] {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reclaimer = Arc::new(HeldReclaimer {
            entered: entered_tx,
            release: Mutex::new(release_rx),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let old_policy = ResidencyPolicy::new(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(old_policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "baseline".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        let request_broker = broker.clone();
        let request = std::thread::spawn(move || {
            request_broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "request".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                30,
            )
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).expect("hard callback entered");
        drop(baseline);
        broker
            .try_live_reconfigure(
                ResidencyPolicy::new(150).with_class_budget(
                    MemoryClass::ArtifactCache,
                    class_budget(0, 150, 150, false),
                ),
            )
            .expect("raise policy while callback is held");
        let competing = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "competing".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                competing_bytes,
            )
            .expect("other thread claims the newly available slot");
        release_tx.send(()).expect("release callback");
        let result = request.join().expect("request thread");
        assert_eq!(result.is_ok(), should_admit, "live postimage decides admission");
        let observation = broker.observe().expect("consistent observation");
        assert_eq!(observation.report.policy_revision, 2);
        assert_eq!(observation.semantic_bytes, competing_bytes + if should_admit { 30 } else { 0 });
        assert_eq!(reclaimer.calls.load(AtomicOrdering::SeqCst), 1);
        drop(result);
        drop(competing);
        sep23_assert_fully_drained(&broker);
    }
}
#[test]
fn concurrent_recharge_and_observe_never_expose_torn_axes() {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::Duration;

    let policy = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false))
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false));
    let broker = LiveProcessMemoryBroker::from_report(report_with(policy)).expect("broker");
    let charge = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "axis:a".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            40,
        )
        .expect("initial charge");
    let done = Arc::new(AtomicBool::new(false));
    let writer_done = done.clone();
    let (stage_tx, stage_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let mut charge = charge;
        for step in 0..1_024 {
            let (class, owner, context, kind) = if step % 2 == 0 {
                (
                    MemoryClass::EvidenceCache,
                    "axis:b".to_string(),
                    MemoryContextKind::Session,
                    ChargeKind::RetainedMetadata,
                )
            } else {
                (
                    MemoryClass::ArtifactCache,
                    "axis:a".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                )
            };
            charge.recharge(class, owner, context, kind, 40).expect("atomic swap");
            if step < 2 {
                stage_tx.send(step).expect("reader observes both live vectors");
                resume_rx.recv().expect("reader releases writer");
            }
            std::thread::yield_now();
        }
        drop(charge);
        writer_done.store(true, AtomicOrdering::Release);
    });
    let inspect = |observation: BrokerObservation| {
        let ledger = &observation.ledger;
        ledger.validate_exact().expect("all accounting axes balance");
        let a = ledger.owner_usage.get("axis:a").map_or(0, |usage| usage.total());
        let b = ledger.owner_usage.get("axis:b").map_or(0, |usage| usage.total());
        assert!(
            (a, b) == (40, 0) || (a, b) == (0, 40) || (a, b) == (0, 0),
            "no mixed owner vector: {ledger:?}"
        );
        let class_a = ledger.class_usage.get(&MemoryClass::ArtifactCache);
        let class_b = ledger.class_usage.get(&MemoryClass::EvidenceCache);
        let context_a = ledger.context_usage.get(&MemoryContextKind::Request);
        let context_b = ledger.context_usage.get(&MemoryContextKind::Session);
        assert_eq!(class_a.map_or(0, |usage| usage.retained_payload_bytes), a);
        assert_eq!(class_a.map_or(0, |usage| usage.retained_metadata_bytes), 0);
        assert_eq!(class_b.map_or(0, |usage| usage.retained_metadata_bytes), b);
        assert_eq!(class_b.map_or(0, |usage| usage.retained_payload_bytes), 0);
        assert_eq!(context_a.map_or(0, |usage| usage.retained_payload_bytes), a);
        assert_eq!(context_a.map_or(0, |usage| usage.retained_metadata_bytes), 0);
        assert_eq!(context_b.map_or(0, |usage| usage.retained_metadata_bytes), b);
        assert_eq!(context_b.map_or(0, |usage| usage.retained_payload_bytes), 0);
        assert_eq!(ledger.live_ticket_count, usize::from(a + b != 0));
        assert_eq!(observation.semantic_bytes, a + b);
        (a, b)
    };
    // The first two handshakes guarantee that both nonzero postimages are
    // actually observed while the writer thread is live; the remaining
    // swaps race with the reader without timing-based assertions.
    for expected in [(0, 40), (40, 0)] {
        stage_rx.recv_timeout(Duration::from_secs(5)).expect("writer reached next vector");
        assert!(!done.load(AtomicOrdering::Acquire));
        assert_eq!(inspect(broker.observe().expect("staged observation")), expected);
        resume_tx.send(()).expect("resume writer");
    }
    let mut samples = 0usize;
    while !done.load(AtomicOrdering::Acquire) || samples < 64 {
        inspect(broker.observe().expect("lock-consistent observation"));
        samples += 1;
    }
    writer.join().expect("writer");
    assert_eq!(inspect(broker.observe().expect("drained observation")), (0, 0));
    sep23_assert_fully_drained(&broker);
}
#[test]
fn forged_snapshot_has_fallible_totals_without_panicking() {
    let mut snapshot = LedgerSnapshot::default();
    snapshot.owner_usage.insert(
        "a".into(),
        crate::UsageBreakdown {
            retained_payload_bytes: u64::MAX,
            retained_metadata_bytes: 0,
            live_scratch_bytes: 0,
        },
    );
    snapshot.owner_usage.insert(
        "b".into(),
        crate::UsageBreakdown {
            retained_payload_bytes: 1,
            retained_metadata_bytes: 0,
            live_scratch_bytes: 0,
        },
    );
    assert!(matches!(snapshot.checked_total_owner_bytes(), Err(MemoryError::InvalidState(_))));
}
#[test]
fn concurrent_reserve_and_reconfigure_preserve_group_cap_truth() {
    use crate::domain::policy::ResourceGroup;

    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50_000,
        hard_cap_bytes: 100_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let residency = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 300_000, false))
        .with_resource_group("g", group.clone())
        .with_owner_group("owner:g", "g");
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
    let baseline = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            90_000,
        )
        .expect("baseline charge");

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let reconfigure_broker = b.clone();
    let reconfigure_barrier = barrier.clone();
    let reconfigure_thread = std::thread::spawn(move || {
        reconfigure_barrier.wait();
        for i in 0..128u64 {
            let raised = ResidencyPolicy::new(20_000_000 + i)
                .with_class_budget(
                    MemoryClass::ArtifactCache,
                    class_budget(0, 200_000, 400_000, false),
                )
                .with_resource_group("g", group.clone())
                .with_owner_group("owner:g", "g");
            reconfigure_broker.try_live_reconfigure(raised).expect("accepted raise");
        }
    });

    let reserve_broker = b.clone();
    let reserve_barrier = barrier.clone();
    let reserve_thread = std::thread::spawn(move || {
        reserve_barrier.wait();
        let mut denied = 0usize;
        let mut admitted = 0usize;
        for _ in 0..128 {
            match reserve_broker.reserve(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                20_000,
            ) {
                Err(MemoryError::Capacity(_)) => denied += 1,
                Ok(ticket) => {
                    admitted += 1;
                    reserve_broker
                        .release(&ticket)
                        .expect("unexpected admission must be releasable");
                }
                Err(other) => {
                    panic!("unexpected reserve error during concurrent test: {other:?}")
                }
            }
        }
        (denied, admitted)
    });

    barrier.wait();
    reconfigure_thread.join().expect("reconfigure thread");
    let (denied, admitted) = reserve_thread.join().expect("reserve thread");
    assert_eq!(admitted, 0, "group overflow reserve must never be admitted");
    assert_eq!(denied, 128, "every concurrent overflow reserve must be denied");
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 90_000, "baseline usage must survive unchanged");
    assert!(snap.class_owner_balanced(), "ledger invariants must survive concurrency");
    b.release(&baseline).expect("release baseline");
}
#[test]
fn soft_cap_crossing_triggers_reclaim_without_immediate_hard_deny() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 50, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    *reclaimer.broker_for_reentry.lock().expect("broker") = Some(broker.clone());

    let charge = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "soft-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            60,
        )
        .expect("soft crossing remains admissible below hard");
    let requests = reclaimer.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].trigger, ReclaimTrigger::SoftThreshold);
    assert_eq!(requests[0].max_attempts, 1);
    drop(requests);
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
    drop(charge);
}
