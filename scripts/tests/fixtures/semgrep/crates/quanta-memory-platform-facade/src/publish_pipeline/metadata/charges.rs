fn persistent(accounting: Accounting) -> Result<(), Error> {
    // ruleid: qmp-persistent-publication-metadata-kind
    let lease = accounting.charge(class, owner, context, ChargeKind::RetainedPayload, persistent_bytes)?;
    Ok(())
}
fn active(accounting: Accounting) -> Result<(), Error> {
    let lease = accounting.charge(class, owner, context, ChargeKind::RetainedPayload, active_bytes)?;
    Ok(())
}
