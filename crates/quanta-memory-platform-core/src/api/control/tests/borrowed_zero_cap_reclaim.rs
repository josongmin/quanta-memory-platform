use super::*;

#[derive(Default)]
struct CandidateAwareBorrowReclaimer {
    requests: Mutex<Vec<ReclaimRequest>>,
    victim: Mutex<Option<ManagedCharge>>,
}

impl MemoryReclaimer for CandidateAwareBorrowReclaimer {
    fn reclaim(
        &self,
        request: &ReclaimRequest,
    ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
        self.requests.lock().expect("request log").push(request.clone());
        if request.candidates.iter().any(|candidate| candidate.class == MemoryClass::SpillFileCache)
        {
            let victim = self.victim.lock().expect("victim slot").take();
            if let Some(mut victim) = victim {
                let reclaimed_bytes = victim.bytes().expect("live victim charge");
                victim.release()?;
                return Ok(crate::ports::reclaim::ReclaimOutcome { reclaimed_bytes });
            }
        }
        Ok(crate::ports::reclaim::ReclaimOutcome::default())
    }
}

#[test]
fn positive_borrow_from_zero_hard_cap_is_a_victim_for_other_class_reclaim() {
    let zero_borrow = class_budget(0, 0, 0, true);
    let policy = ResidencyPolicy::new(100)
        .with_shared_burst_pool_bytes(80)
        .with_class_budget(MemoryClass::SpillFileCache, zero_borrow.clone())
        .with_class_budget(MemoryClass::EvidenceCache, zero_borrow.clone())
        .with_class_budget(MemoryClass::ParserResidency, zero_borrow);
    let reclaimer = Arc::new(CandidateAwareBorrowReclaimer::default());
    let broker = crate::ProcessMemoryBrokerBuilder::new()
        .with_manifest(crate::MemoryBudgetManifest::new(
            crate::MemoryPreset::Development,
            crate::ExecutionBudget::default(),
            policy,
        ))
        .with_reclaimer(reclaimer.clone())
        .build()
        .expect("zero hard cap may borrow from a positive shared pool");
    let victim = broker
        .reserve_managed_charge(
            MemoryClass::SpillFileCache,
            "owner:victim".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            60,
        )
        .expect("victim borrows 60 bytes");
    *reclaimer.victim.lock().expect("victim slot") = Some(victim);

    let admitted = broker
        .reserve_managed_charge(
            MemoryClass::EvidenceCache,
            "owner:requester".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        )
        .expect("candidate-aware callback releases the actual pool victim");
    let requests = reclaimer.requests.lock().expect("request log");
    let hard_request = requests.last().expect("hard reclaim request");
    assert_eq!(hard_request.trigger, ReclaimTrigger::CapacityDenial);
    assert_eq!(hard_request.requested_class, MemoryClass::EvidenceCache);
    assert!(hard_request
        .candidates
        .iter()
        .any(|candidate| candidate.class == MemoryClass::SpillFileCache));
    assert!(!hard_request
        .candidates
        .iter()
        .any(|candidate| candidate.class == MemoryClass::ParserResidency));
    drop(requests);
    assert!(reclaimer.victim.lock().expect("released victim slot").is_none());
    assert_eq!(admitted.bytes(), Some(30));
    let snapshot = broker.ledger_snapshot().expect("remeasured ledger");
    assert_eq!(snapshot.total_owner_bytes(), 30);
    assert_eq!(snapshot.class_usage.get(&MemoryClass::SpillFileCache), None);
    assert_eq!(
        snapshot.class_usage.get(&MemoryClass::EvidenceCache).map(|usage| usage.total()),
        Some(30)
    );
    drop(admitted);
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").live_ticket_count, 0);
}
