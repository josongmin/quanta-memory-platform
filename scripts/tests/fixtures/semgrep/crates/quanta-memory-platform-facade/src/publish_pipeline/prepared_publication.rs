fn bad(accounting: Accounting) {
    // ruleid: qmp-persistent-publication-metadata-kind
    let persistent_charge = accounting.charge(class, owner, context, ChargeKind::RetainedPayload, persistent_bytes).map_err(Error::Accounting);
}
fn good(accounting: Accounting) {
    let persistent_charge = accounting.charge(class, owner, context, ChargeKind::RetainedMetadata, persistent_bytes).map_err(Error::Accounting);
    let active_charge = accounting.charge(class, owner, context, ChargeKind::RetainedPayload, active_bytes).map_err(Error::Accounting);
}
