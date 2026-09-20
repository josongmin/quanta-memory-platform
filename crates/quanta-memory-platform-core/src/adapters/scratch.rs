//! Broker-backed implementation of the scratch-allocation port.
//!
//! A scratch scope is a capacity permit: it reserves the execution-scratch
//! budget before the caller allocates memory and owns that pending authority
//! until explicit release or drop. It deliberately does not create a second
//! counter beside the broker ledger.

use crate::api::control::{LiveProcessMemoryBroker, PendingReservation};
use crate::domain::context::{MemoryContextHandle, RuntimeId};
use crate::domain::errors::MemoryError;
use crate::domain::ledger::ChargeKind;
use crate::domain::policy::MemoryClass;
use crate::ports::scratch::{
    ScratchAllocator, ScratchRequest, ScratchReservation, ScratchReservationAuthority,
};
use crate::{CapacityError, MemoryResult};

struct BrokerScratchAuthority {
    pending: PendingReservation,
}

impl ScratchReservationAuthority for BrokerScratchAuthority {
    fn release(&mut self) -> MemoryResult<()> {
        self.pending.rollback()
    }
}

#[derive(Clone)]
pub(crate) struct BrokerBoundScratchAllocator {
    broker: LiveProcessMemoryBroker,
    runtime_id: RuntimeId,
}

impl BrokerBoundScratchAllocator {
    pub(crate) fn new(broker: LiveProcessMemoryBroker, runtime_id: RuntimeId) -> Self {
        Self { broker, runtime_id }
    }
}

impl ScratchAllocator for BrokerBoundScratchAllocator {
    fn reserve_scratch_scope(
        &self,
        ctx: &MemoryContextHandle,
        request: ScratchRequest,
    ) -> Result<ScratchReservation, CapacityError> {
        if ctx.runtime_id != self.runtime_id {
            return Err(MemoryError::InvalidState(format!(
                "scratch context runtime {} does not match attached runtime {}",
                ctx.runtime_id, self.runtime_id
            )));
        }

        let pending = self.broker.reserve_owned(
            MemoryClass::ExecutionScratch,
            ctx.owner.clone(),
            ctx.kind,
            ChargeKind::LiveScratch,
            request.requested_bytes,
        )?;
        Ok(ScratchReservation::enforced(
            ctx.clone(),
            request.label,
            request.requested_bytes,
            Box::new(BrokerScratchAuthority { pending }),
        ))
    }
}
