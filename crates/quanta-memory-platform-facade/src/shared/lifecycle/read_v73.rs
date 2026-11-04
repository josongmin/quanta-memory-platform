use super::*;

impl StoreLifecycle {
    /// Run the same preflight, nonblocking lock, post-lock check and unwind
    /// containment as `with_read_state`. The refusal constructor runs only
    /// after the state guard is released; a caller may move a prepared error.
    pub(crate) fn with_read_state_with_refusal_v73<T, R, E>(
        &self,
        state: &Mutex<T>,
        read: impl FnOnce(&T) -> Result<R, E>,
        refusal: impl FnOnce(RetainedMapReadRefusalV73) -> MemoryError,
    ) -> Result<Result<R, E>, MemoryError> {
        let attempt = (|| {
            let readable = || match self.phase() {
                StoreLifecyclePhase::Open | StoreLifecyclePhase::Closed => Ok(()),
                StoreLifecyclePhase::OperationInProgress | StoreLifecyclePhase::Closing => {
                    Err(RetainedMapReadRefusalV73::OperationInProgress { resource: self.resource })
                }
            };
            readable()?;
            let guard = state.try_lock().map_err(|cause| match cause {
                TryLockError::WouldBlock => {
                    RetainedMapReadRefusalV73::OperationInProgress { resource: self.resource }
                }
                TryLockError::Poisoned(_) => {
                    RetainedMapReadRefusalV73::Poisoned { resource: self.resource }
                }
            })?;
            readable()?;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(&guard)));
            drop(guard);
            match result {
                Ok(value) => Ok(value),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        })();
        attempt.map_err(refusal)
    }
}
