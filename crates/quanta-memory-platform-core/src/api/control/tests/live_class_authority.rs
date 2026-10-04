use super::*;

fn broker_with_optional_zero_class() -> LiveProcessMemoryBroker {
    let policy = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::SpillFileCache, class_budget(0, 0, 0, false));
    crate::ProcessMemoryBrokerBuilder::new()
        .with_manifest(crate::MemoryBudgetManifest::new(
            crate::MemoryPreset::Development,
            crate::ExecutionBudget::default(),
            policy,
        ))
        .build()
        .expect("optional zero-cap class is valid")
}

#[test]
fn pending_zero_byte_optional_class_blocks_removal_until_commit_and_drop() {
    let broker = broker_with_optional_zero_class();
    let pending = broker
        .reserve_owned(
            MemoryClass::SpillFileCache,
            "owner:pending".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("zero-byte pending ticket is a live authority");
    let before_report = broker.effective_report().expect("report before denial");
    let before_ledger = broker.ledger_snapshot().expect("ledger before denial");
    assert_eq!(before_ledger.live_ticket_count, 1);
    assert_eq!(before_ledger.total_owner_bytes(), 0);

    let denied = broker.try_live_reconfigure(ResidencyPolicy::new(100));
    assert!(matches!(denied, Err(MemoryError::InvalidState(_))));
    assert_eq!(broker.effective_report().expect("report after denial"), before_report);
    assert_eq!(broker.ledger_snapshot().expect("ledger after denial"), before_ledger);

    let charge = pending.commit().expect("denied removal preserves pending authority");
    assert_eq!(charge.class(), Some(&MemoryClass::SpillFileCache));
    drop(charge);
    broker.try_live_reconfigure(ResidencyPolicy::new(100)).expect("class removal after drain");
    assert!(!broker
        .effective_report()
        .expect("installed report")
        .residency
        .classes
        .contains_key(&MemoryClass::SpillFileCache));
}

#[test]
fn committed_zero_byte_optional_class_blocks_removal_until_explicit_release() {
    let broker = broker_with_optional_zero_class();
    let mut charge = broker
        .reserve_managed_charge(
            MemoryClass::SpillFileCache,
            "owner:committed".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("zero-byte committed ticket is a live authority");
    let before_report = broker.effective_report().expect("report before denial");
    let before_ledger = broker.ledger_snapshot().expect("ledger before denial");
    assert_eq!(before_ledger.live_ticket_count, 1);
    assert_eq!(before_ledger.total_owner_bytes(), 0);

    assert!(matches!(
        broker.try_live_reconfigure(ResidencyPolicy::new(100)),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(broker.effective_report().expect("report after denial"), before_report);
    assert_eq!(broker.ledger_snapshot().expect("ledger after denial"), before_ledger);

    charge.release().expect("denied removal preserves release authority");
    assert!(!charge.is_active());
    broker.try_live_reconfigure(ResidencyPolicy::new(100)).expect("class removal after release");
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").live_ticket_count, 0);
}

#[test]
fn zero_byte_class_ticket_allows_cap_change_while_class_remains_configured() {
    let broker = broker_with_optional_zero_class();
    let charge = broker
        .reserve_managed_charge(
            MemoryClass::SpillFileCache,
            "owner:retained".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("zero-byte class ticket");
    let raised = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::SpillFileCache, class_budget(0, 1, 1, false));
    broker.try_live_reconfigure(raised).expect("class remains configured");
    assert_eq!(broker.ledger_snapshot().expect("ticket still live").live_ticket_count, 1);
    drop(charge);
    assert_eq!(broker.ledger_snapshot().expect("ticket drained").live_ticket_count, 0);
}
