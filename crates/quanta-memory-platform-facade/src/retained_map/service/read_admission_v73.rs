use super::*;

impl<Key, Value, Mode> SimpleRetainedMapStore<Key, Value, Mode>
where
    Key: Clone + Eq + Hash + ByteSized,
    Value: Clone + ByteSized,
{
    /// Checked point read with a caller-funded lookup bound. The callback
    /// observes the exact entry count under the read lock before any key hash,
    /// equality, or value clone. Lifecycle refusal remains `MemoryError`.
    pub fn get_checked_with_admission_v73<E>(
        &self,
        key: &Key,
        before_lookup: impl FnOnce(usize) -> Result<(), E>,
        refusal: impl FnOnce(RetainedMapReadRefusalV73) -> MemoryError,
    ) -> Result<Result<Option<Value>, E>, MemoryError> {
        self.lifecycle.with_read_state_with_refusal_v73(
            &self.state,
            |state| {
                before_lookup(state.entries.len())?;
                Ok(state.entries.get(key).map(|entry| entry.value.clone()))
            },
            refusal,
        )
    }
}
