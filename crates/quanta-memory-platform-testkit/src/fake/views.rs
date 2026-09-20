//! Fake runtime-view substrate.
//!
//! This fake deliberately rejects scratch allocation: successful scratch
//! authority belongs to the live broker and must not be reimplemented here.

use quanta_memory_platform_core::{
    MemoryClass, MemoryContextHandle, MemoryContextKind, MemoryError, OwnerKey, RuntimeId,
    RuntimeKind, RuntimeMemoryView, ScratchAllocator, ScratchRequest, ScratchReservation,
    StoreBindingHandle,
};

#[derive(Debug, Default)]
struct RejectingScratchAllocator;

impl ScratchAllocator for RejectingScratchAllocator {
    fn reserve_scratch_scope(
        &self,
        _ctx: &MemoryContextHandle,
        _request: ScratchRequest,
    ) -> Result<ScratchReservation, MemoryError> {
        Err(MemoryError::Unsupported("fake runtime view carries no scratch authority".to_string()))
    }
}

#[derive(Debug)]
pub struct FakeRuntimeMemoryView {
    runtime_id: RuntimeId,
    runtime_kind: RuntimeKind,
    scratch: RejectingScratchAllocator,
}

impl RuntimeMemoryView for FakeRuntimeMemoryView {
    fn runtime_id(&self) -> &RuntimeId {
        &self.runtime_id
    }

    fn runtime_kind(&self) -> RuntimeKind {
        self.runtime_kind
    }

    fn context(&self, kind: MemoryContextKind, owner: OwnerKey) -> MemoryContextHandle {
        MemoryContextHandle::new(self.runtime_id.clone(), kind, owner)
    }

    fn bind_store(&self, class: MemoryClass, owner: OwnerKey) -> StoreBindingHandle {
        StoreBindingHandle::new(class, owner, Some(self.runtime_id.clone()))
    }

    fn scratch(&self) -> &dyn ScratchAllocator {
        &self.scratch
    }
}

pub struct FakeRuntimeViewFactory;

impl FakeRuntimeViewFactory {
    /// A generic fake runtime view. The runtime id is a stable test label, not a
    /// host identifier.
    #[must_use]
    pub fn runtime_view() -> FakeRuntimeMemoryView {
        Self::runtime_view_of(RuntimeKind::Runtime)
    }

    /// A fake runtime view of a chosen portable kind.
    #[must_use]
    pub fn runtime_view_of(kind: RuntimeKind) -> FakeRuntimeMemoryView {
        FakeRuntimeMemoryView {
            runtime_id: "fake-runtime".to_string(),
            runtime_kind: kind,
            scratch: RejectingScratchAllocator,
        }
    }
}
