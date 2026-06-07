use crate::{MemoryClass, MemoryContextHandle, OwnerKey, RuntimeId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreBindingHandle {
    pub class: MemoryClass,
    pub owner: OwnerKey,
    pub runtime_id: Option<RuntimeId>,
}

impl StoreBindingHandle {
    pub fn new(class: MemoryClass, owner: OwnerKey, runtime_id: Option<RuntimeId>) -> Self {
        Self { class, owner, runtime_id }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreStats {
    pub item_count: usize,
    pub estimated_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedEnvelope<Bytes> {
    pub bytes: Bytes,
    pub context: Option<MemoryContextHandle>,
}

impl<Bytes> RetainedEnvelope<Bytes> {
    pub fn new(bytes: Bytes) -> Self {
        Self { bytes, context: None }
    }

    pub fn with_context(mut self, context: MemoryContextHandle) -> Self {
        self.context = Some(context);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedPayload<PayloadRef> {
    pub payload_ref: PayloadRef,
}

impl<PayloadRef> PublishedPayload<PayloadRef> {
    pub fn new(payload_ref: PayloadRef) -> Self {
        Self { payload_ref }
    }

    pub fn payload_ref(&self) -> &PayloadRef {
        &self.payload_ref
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SweepHint {
    pub target_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SweepStats {
    pub reclaimed_items: usize,
    pub reclaimed_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedHandle<Stage> {
    pub staged: Stage,
}

impl<Stage> StagedHandle<Stage> {
    pub fn new(staged: Stage) -> Self {
        Self { staged }
    }
}
