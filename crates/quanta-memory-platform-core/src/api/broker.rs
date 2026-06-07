use std::sync::Arc;

use crate::adapters::pin::BasicPinRegistry;
use crate::adapters::scratch::BasicScratchAllocator;
use crate::domain::context::{
    MemoryContextHandle, MemoryContextKind, OwnerKey, RuntimeId, RuntimeKind,
};
use crate::domain::stats::EffectiveMemoryBudgetReport;
use crate::ports::pin::PinRegistry;
use crate::ports::scratch::ScratchAllocator;
use crate::ports::store::StoreBindingHandle;
use crate::{MemoryClass, MemoryResult};

pub trait RuntimeMemoryView: Send + Sync {
    fn runtime_id(&self) -> &RuntimeId;
    fn runtime_kind(&self) -> RuntimeKind;
    fn context(&self, kind: MemoryContextKind, owner: OwnerKey) -> MemoryContextHandle;
    fn bind_store(&self, class: MemoryClass, owner: OwnerKey) -> StoreBindingHandle;
    fn scratch(&self) -> &dyn ScratchAllocator;
    fn pins(&self) -> &dyn PinRegistry<String, String>;
}

pub trait ProcessMemoryBroker: Send + Sync {
    fn attach_runtime(
        &self,
        runtime_id: RuntimeId,
        runtime_kind: RuntimeKind,
    ) -> MemoryResult<Arc<dyn RuntimeMemoryView>>;

    fn effective_report(&self) -> EffectiveMemoryBudgetReport;
}

#[derive(Clone, Debug)]
pub struct BasicRuntimeMemoryView {
    runtime_id: RuntimeId,
    runtime_kind: RuntimeKind,
    scratch: BasicScratchAllocator,
    pins: BasicPinRegistry,
}

impl BasicRuntimeMemoryView {
    pub fn new(runtime_id: RuntimeId, runtime_kind: RuntimeKind) -> Self {
        Self {
            runtime_id,
            runtime_kind,
            scratch: BasicScratchAllocator,
            pins: BasicPinRegistry::default(),
        }
    }
}

impl RuntimeMemoryView for BasicRuntimeMemoryView {
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

    fn pins(&self) -> &dyn PinRegistry<String, String> {
        &self.pins
    }
}

#[derive(Clone, Debug)]
pub struct BasicProcessMemoryBroker {
    report: EffectiveMemoryBudgetReport,
}

impl BasicProcessMemoryBroker {
    pub fn new(report: EffectiveMemoryBudgetReport) -> Self {
        Self { report }
    }
}

impl ProcessMemoryBroker for BasicProcessMemoryBroker {
    fn attach_runtime(
        &self,
        runtime_id: RuntimeId,
        runtime_kind: RuntimeKind,
    ) -> MemoryResult<Arc<dyn RuntimeMemoryView>> {
        Ok(Arc::new(BasicRuntimeMemoryView::new(runtime_id, runtime_kind)))
    }

    fn effective_report(&self) -> EffectiveMemoryBudgetReport {
        self.report.clone()
    }
}
