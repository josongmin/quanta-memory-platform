//! Fake runtime-view substrate.
//!
//! A thin factory over the core `BasicRuntimeMemoryView` so conformance and
//! fault fixtures can obtain a runtime view without a host runtime. This stays
//! host-independent: it only names the portable `RuntimeKind`, never a Semantica
//! runtime noun.

use quanta_memory_platform_core::{BasicRuntimeMemoryView, RuntimeKind};

pub struct FakeRuntimeViewFactory;

impl FakeRuntimeViewFactory {
    /// A generic fake runtime view. The runtime id is a stable test label, not a
    /// host identifier.
    #[must_use]
    pub fn runtime_view() -> BasicRuntimeMemoryView {
        BasicRuntimeMemoryView::new("fake-runtime".to_string(), RuntimeKind::Runtime)
    }

    /// A fake runtime view of a chosen portable kind.
    #[must_use]
    pub fn runtime_view_of(kind: RuntimeKind) -> BasicRuntimeMemoryView {
        BasicRuntimeMemoryView::new("fake-runtime".to_string(), kind)
    }
}
