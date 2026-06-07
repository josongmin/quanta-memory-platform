//! The pin-registry PORT: a pure trait surface plus its contract types. The
//! concrete in-memory adapter lives in `crate::adapters::pin::BasicPinRegistry`
//! so the port layer carries no `std::sync`/storage machinery (hexagonal rule).

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
