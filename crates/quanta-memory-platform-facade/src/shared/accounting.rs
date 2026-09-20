//! Sealed accounting modes for facade stores.
//!
//! Production stores never invoke a user-provided callback. A budgeted builder
//! carries the concrete live broker that issued every entry's non-clone
//! [`ManagedCharge`]. This fixes one global lock order (`store -> broker`) and
//! prevents reentrant accounting callbacks from entering a locked store.

#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, MemoryClass, MemoryContextKind,
    MemoryResult, OwnerKey,
};

/// Builder state before accounting policy has been selected.
///
/// This state intentionally has no `build` method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccountingUnset {
    _private: (),
}

/// Production accounting mode. Values are issued only by [`LiveMemoryPlatform`]
/// store factories; consumers cannot fabricate or rebind this authority.
///
/// [`LiveMemoryPlatform`]: crate::shared::builder::LiveMemoryPlatform
#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
#[derive(Clone)]
pub struct Budgeted {
    broker: LiveProcessMemoryBroker,
}

#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
impl std::fmt::Debug for Budgeted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Budgeted").finish_non_exhaustive()
    }
}

#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
impl Budgeted {
    pub(crate) fn new(broker: LiveProcessMemoryBroker) -> Self {
        Self { broker }
    }

    pub(crate) fn into_broker(self) -> LiveProcessMemoryBroker {
        self.broker
    }
}

/// Explicit escape hatch for fixtures and consumers that intentionally opt out
/// of ledger enforcement. It exists only with the non-default
/// `unbudgeted-stores` feature and is a different product type from [`Budgeted`].
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{
///     AccountingUnset, Budgeted, PayloadStoreBuilder, SimplePayloadStore,
/// };
/// use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
///
/// fn requires_budgeted(_: SimplePayloadStore<String, Vec<u8>, Budgeted>) {}
/// let binding = StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner".into(), None);
/// let unbudgeted = PayloadStoreBuilder::<String, Vec<u8>, AccountingUnset>::new()
///     .unbudgeted()
///     .bind(binding)
///     .build()
///     .unwrap();
/// requires_budgeted(unbudgeted);
/// ```
#[cfg(all(
    feature = "unbudgeted-stores",
    any(
        feature = "payload_store",
        feature = "retained_map",
        feature = "revocable_store",
        feature = "transcript",
        feature = "publish_pipeline"
    )
))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unbudgeted {
    _private: (),
}

#[cfg(all(
    feature = "unbudgeted-stores",
    any(
        feature = "payload_store",
        feature = "retained_map",
        feature = "revocable_store",
        feature = "transcript",
        feature = "publish_pipeline"
    )
))]
impl Unbudgeted {
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }
}

/// Concrete, crate-owned accounting authority embedded in every store.
#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
#[derive(Clone, Debug)]
pub(crate) enum StoreAccounting {
    Budgeted(LiveProcessMemoryBroker),
    #[cfg(any(
        feature = "unbudgeted-stores",
        all(
            test,
            any(feature = "retained_map", feature = "transcript", feature = "publish_pipeline")
        )
    ))]
    Unbudgeted,
}

#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
impl StoreAccounting {
    pub(crate) fn charge(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<Option<ManagedCharge>> {
        match self {
            Self::Budgeted(broker) => {
                broker.reserve_managed_charge(class, owner, context, kind, bytes).map(Some)
            }
            #[cfg(any(
                feature = "unbudgeted-stores",
                all(
                    test,
                    any(
                        feature = "retained_map",
                        feature = "transcript",
                        feature = "publish_pipeline"
                    )
                )
            ))]
            Self::Unbudgeted => Ok(None),
        }
    }

    pub(crate) fn release_many(&self, charges: &mut [ManagedCharge]) -> MemoryResult<()> {
        match self {
            Self::Budgeted(broker) => broker.release_managed_many(charges),
            #[cfg(any(
                feature = "unbudgeted-stores",
                all(
                    test,
                    any(
                        feature = "retained_map",
                        feature = "transcript",
                        feature = "publish_pipeline"
                    )
                )
            ))]
            Self::Unbudgeted => {
                if charges.is_empty() {
                    Ok(())
                } else {
                    Err(quanta_memory_platform_core::MemoryError::InvalidState(
                        "unbudgeted store cannot own managed charges".to_string(),
                    ))
                }
            }
        }
    }

    #[cfg(any(
        feature = "retained_map",
        feature = "revocable_store",
        feature = "transcript",
        feature = "publish_pipeline"
    ))]
    pub(crate) fn broker(&self) -> Option<&LiveProcessMemoryBroker> {
        match self {
            Self::Budgeted(broker) => Some(broker),
            #[cfg(any(
                feature = "unbudgeted-stores",
                all(
                    test,
                    any(
                        feature = "retained_map",
                        feature = "transcript",
                        feature = "publish_pipeline"
                    )
                )
            ))]
            Self::Unbudgeted => None,
        }
    }
}
