//! Concrete adapter for the [`ScratchAllocator`] port.
//!
//! Lives in `adapters/` so the port layer is a pure trait + value-type surface;
//! the allocator implementation belongs here (hexagonal rule).

use crate::ports::scratch::{ScratchAllocator, ScratchRequest, ScratchReservation};
use crate::{CapacityError, MemoryContextHandle};

/// The default scratch allocator: hands out a reservation sized exactly to the
/// request. Accounting/enforcement is the broker's responsibility; this adapter
/// only materializes the [`ScratchReservation`] value.
#[derive(Clone, Debug, Default)]
pub struct BasicScratchAllocator;

impl ScratchAllocator for BasicScratchAllocator {
    fn reserve_scratch_scope(
        &self,
        ctx: &MemoryContextHandle,
        request: ScratchRequest,
    ) -> Result<ScratchReservation, CapacityError> {
        Ok(ScratchReservation {
            context: ctx.clone(),
            label: request.label,
            reserved_bytes: request.requested_bytes,
            observed_bytes: 0,
            released: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChargeKind, MemoryContextKind, OwnerKey, RuntimeId};

    #[test]
    fn basic_allocator_reserves_a_live_scratch_scope_sized_to_the_request() {
        let ctx = MemoryContextHandle::new(
            RuntimeId::from("rt"),
            MemoryContextKind::Request,
            OwnerKey::from("owner"),
        );
        let res = BasicScratchAllocator
            .reserve_scratch_scope(&ctx, ScratchRequest::new("buf", 4096))
            .expect("basic allocator reserves");
        assert_eq!(res.reserved_bytes, 4096, "reservation mirrors the requested bytes");
        assert_eq!(res.observed_bytes, 0, "fresh reservation observes nothing yet");
        assert!(!res.released, "fresh reservation is live");
        assert_eq!(res.charge_kind(), ChargeKind::LiveScratch, "scratch is LiveScratch");
    }
}
