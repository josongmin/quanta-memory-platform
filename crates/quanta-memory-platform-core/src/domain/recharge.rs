use crate::domain::errors::MemoryError;

/// Accounting disposition after a denied atomic recharge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RechargeRecoveryV1 {
    /// The old charge and its original ticket remain live and unchanged.
    OldChargePreserved,
}

/// A rejected recharge together with the exact disposition of the old charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RechargeError {
    pub(crate) error: MemoryError,
    pub(crate) recovery: RechargeRecoveryV1,
}
