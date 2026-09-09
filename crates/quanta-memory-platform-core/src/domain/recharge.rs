use crate::domain::errors::MemoryError;
use crate::domain::ledger::ReservationTicket;

/// Accounting disposition after a denied atomic recharge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RechargeRecoveryV1 {
    /// The old charge and its original ticket remain live and unchanged.
    OldChargePreserved,
    /// The old charge was restored under a new ticket, which the caller must adopt.
    OldChargeRestored(ReservationTicket),
    /// The old charge was released and could not be restored. The caller must
    /// remove or quarantine the corresponding retained payload.
    RecoveryRequired { rollback_error: MemoryError },
}

/// A rejected recharge together with the exact disposition of the old charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RechargeError {
    pub error: MemoryError,
    pub recovery: RechargeRecoveryV1,
}
