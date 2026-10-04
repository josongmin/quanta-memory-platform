use super::*;
use quanta_memory_platform_facade::{SpillFailure, SpillInventory, SpillReceipt};

struct EmptyInsteadOfOffline;

impl SpillInventory for EmptyInsteadOfOffline {
    fn retained_count(&self) -> Result<u64, MemoryError> {
        Ok(1)
    }
}

impl SpillTarget<String> for EmptyInsteadOfOffline {
    fn is_available(&self) -> bool {
        false
    }

    fn accept(&self, handle: String) -> Result<SpillReceipt, SpillFailure<String>> {
        Err(SpillFailure::new(MemoryError::capacity("offline"), handle))
    }

    fn restore(&self) -> Result<Option<String>, MemoryError> {
        Ok(None)
    }
}

#[test]
fn offline_restore_conformance_rejects_empty_masking_and_accepts_preserved_inventory() {
    let retained = InMemorySpillTarget::available();
    retained.accept("kept".to_string()).expect("seed retained handle");
    retained.set_available(false);
    let honest = RoutedSpillController::new(retained);
    assert!(spill_offline_restore_verdict(&honest).held());
    assert_eq!(honest.spilled_count(), Ok(1));

    let broken = RoutedSpillController::new(EmptyInsteadOfOffline);
    let verdict = spill_offline_restore_verdict(&broken);
    assert!(!verdict.held(), "offline must not masquerade as drained: {verdict:?}");
    assert_eq!(broken.spilled_count(), Ok(1));
}
