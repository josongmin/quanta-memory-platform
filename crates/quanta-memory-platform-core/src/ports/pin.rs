//! The pin-registry PORT: a pure trait surface plus its contract types. An
//! explicitly unbudgeted in-memory implementation is available behind the
//! non-default `unbudgeted_pin` feature; this port carries no synchronization
//! or storage machinery.

use crate::PinError;

pub trait PinRegistry<PayloadRef, DependentRef> {
    fn pin(
        &self,
        payload: PayloadRef,
        dependent: DependentRef,
    ) -> Result<PayloadPinLease<PayloadRef, DependentRef>, PinError>;

    fn dependent_count(&self, payload: &PayloadRef) -> u64;

    fn invalidate_dependents_atomically(
        &self,
        payload: &PayloadRef,
    ) -> Result<InvalidationOutcome<DependentRef>, PinError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadPinLease<PayloadRef, DependentRef> {
    pub payload: PayloadRef,
    pub dependent: DependentRef,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InvalidationOutcome<DependentRef> {
    pub invalidated_dependents: Vec<DependentRef>,
}
