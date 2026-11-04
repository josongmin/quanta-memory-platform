use super::*;

impl<Key, Value, Mode> SimpleRetainedMapStore<Key, Value, Mode>
where
    Key: Clone + Eq + Hash + ByteSized,
    Value: Clone + ByteSized,
{
    pub(super) fn committed_bytes_after_replace_v1(
        committed_bytes: u64,
        removed_bytes: u64,
        added_bytes: u64,
    ) -> Result<u64, StoreWriteError> {
        let retained_without_old = committed_bytes.checked_sub(removed_bytes).ok_or_else(|| {
            MemoryError::InvalidState(
                "retained-map committed bytes are smaller than the replaced entry".to_string(),
            )
        })?;
        retained_without_old
            .checked_add(added_bytes)
            .ok_or_else(|| MemoryError::capacity("retained-map committed byte accounting overflow"))
    }
}
