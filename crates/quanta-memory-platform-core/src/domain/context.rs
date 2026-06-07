pub type RuntimeId = String;
pub type OwnerKey = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeKind {
    Runtime,
    Retrieval,
    Session,
    ControlPlane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryContextKind {
    Request,
    Session,
    Generation,
    Runtime,
    Background,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryContextHandle {
    pub runtime_id: RuntimeId,
    pub kind: MemoryContextKind,
    pub owner: OwnerKey,
}

impl MemoryContextHandle {
    pub fn new(runtime_id: RuntimeId, kind: MemoryContextKind, owner: OwnerKey) -> Self {
        Self { runtime_id, kind, owner }
    }
}
