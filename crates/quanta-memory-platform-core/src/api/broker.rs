use std::sync::Arc;

use crate::domain::context::{
    MemoryContextHandle, MemoryContextKind, OwnerKey, RuntimeId, RuntimeKind,
};
use crate::domain::stats::EffectiveMemoryBudgetReport;
use crate::ports::scratch::ScratchAllocator;
use crate::ports::store::StoreBindingHandle;
use crate::{MemoryClass, MemoryResult};

pub trait RuntimeMemoryView: Send + Sync {
    fn runtime_id(&self) -> &RuntimeId;
    fn runtime_kind(&self) -> RuntimeKind;
    fn context(&self, kind: MemoryContextKind, owner: OwnerKey) -> MemoryContextHandle;
    fn bind_store(&self, class: MemoryClass, owner: OwnerKey) -> StoreBindingHandle;
    fn scratch(&self) -> &dyn ScratchAllocator;
}

pub trait ProcessMemoryBroker: Send + Sync {
    fn attach_runtime(
        &self,
        runtime_id: RuntimeId,
        runtime_kind: RuntimeKind,
    ) -> MemoryResult<Arc<dyn RuntimeMemoryView>>;

    fn effective_report(&self) -> EffectiveMemoryBudgetReport;
}
