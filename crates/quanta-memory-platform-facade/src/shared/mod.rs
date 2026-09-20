//! Shared, slice-agnostic glue for the portable facade.
//!
//! Only things used by more than one slice live here: the builder-first happy
//! path ([`builder`]) and the domain-neutral byte-measurement contract
//! ([`measure`]). Slice-local backing integrations (`scc` / `redb` /
//! `moka`) must never be promoted into this module.

pub mod accounting;
pub mod builder;
#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
pub(crate) mod lifecycle;
pub mod measure;

pub use accounting::AccountingUnset;
#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
pub use accounting::Budgeted;
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
pub use accounting::Unbudgeted;
pub use measure::{semantic_cost_v1, ByteSized, CanonicalKey, MeasuredValue, RetainedCostV1};
