use super::super::*;

struct ReadOnlyTarget(Mutex<VecDeque<String>>);

impl SpillInventory for ReadOnlyTarget {
    fn retained_count(&self) -> MemoryResult<u64> {
        Ok(u64::try_from(self.0.lock().expect("fixture queue").len()).expect("small fixture"))
    }
}

impl SpillTarget<String> for ReadOnlyTarget {
    fn is_available(&self) -> bool {
        false
    }

    fn accept(&self, handle: String) -> Result<SpillReceipt, SpillFailure<String>> {
        Err(SpillFailure::new(MemoryError::capacity("write admission is full"), handle))
    }

    fn restore(&self) -> MemoryResult<Option<String>> {
        Ok(self.0.lock().expect("fixture queue").pop_front())
    }
}

#[test]
fn write_full_readable_target_restores_without_write_admission() {
    let controller = RoutedSpillController::new(ReadOnlyTarget(Mutex::new(VecDeque::from([
        "oldest".to_string(),
        "newest".to_string(),
    ]))));
    let denied = controller.spill("rejected".to_string()).expect_err("writes are full");
    assert!(matches!(denied.error(), MemoryError::Capacity(_)));
    assert_eq!(denied.into_parts().1, "rejected");
    assert_eq!(controller.spilled_count(), Ok(2));
    assert_eq!(controller.restore(), Ok(Some("oldest".to_string())));
    assert_eq!(controller.restore(), Ok(Some("newest".to_string())));
    assert_eq!(controller.restore(), Ok(None));
    assert_eq!(controller.spilled_count(), Ok(0));
}
