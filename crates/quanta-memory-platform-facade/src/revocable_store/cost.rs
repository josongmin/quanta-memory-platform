//! One semantic-cost formula for resident revocable entries and durable decode
//! admission. The codec may inspect lengths before allocating values, but it
//! must reserve the same charge that the resident store will retain.

use quanta_memory_platform_core::MemoryResult;

use crate::shared::measure::{semantic_cost_v1, RetainedCostV1};

pub(super) fn entry_cost(key_bytes: u64, payload_bytes: u64) -> MemoryResult<u64> {
    RetainedCostV1 {
        payload_bytes,
        key_identity_bytes: key_bytes,
        index_bytes: semantic_cost_v1::FIFO_INDEX_BYTES,
        fixed_overhead_bytes: semantic_cost_v1::ENTRY_STATE_BYTES,
        ..RetainedCostV1::default()
    }
    .total()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quanta_memory_platform_core::MemoryError;

    #[test]
    fn revocable_entry_cost_has_one_checked_formula() {
        assert_eq!(entry_cost(1, 1), Ok(34));
        assert!(matches!(entry_cost(u64::MAX, 1), Err(MemoryError::Capacity(_))));
    }
}
