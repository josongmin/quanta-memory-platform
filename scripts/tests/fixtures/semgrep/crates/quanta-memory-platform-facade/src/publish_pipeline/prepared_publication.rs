fn bad(accounting: Accounting) {
    // ruleid: qmp-persistent-publication-metadata-kind
    let persistent_charge = accounting.charge(class, owner, context, ChargeKind::RetainedPayload, persistent_bytes).map_err(Error::Accounting);
}
fn renamed_without_map_err(accounting: Accounting) -> Result<(), Error> {
    // ruleid: qmp-persistent-publication-metadata-kind
    let lease = accounting.charge(class, owner, context, ChargeKind::RetainedPayload, persistent_bytes)?;
    Ok(())
}
fn good(accounting: Accounting) {
    let persistent_charge = accounting.charge(class, owner, context, ChargeKind::RetainedMetadata, persistent_bytes).map_err(Error::Accounting);
    let active_charge = accounting.charge(class, owner, context, ChargeKind::RetainedPayload, active_bytes).map_err(Error::Accounting);
}
