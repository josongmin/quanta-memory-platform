//! Shared, slice-agnostic glue for the portable facade.
//!
//! Only things used by more than one slice live here: the builder-first happy
//! path ([`builder`]) and the domain-neutral byte-measurement contract
//! ([`measure`]). Slice-local backing integrations (`bumpalo` / `scc` / `redb` /
//! `moka`) must never be promoted into this module.

pub mod accounting;
pub mod builder;
pub mod measure;

pub use accounting::{AccountingSink, SharedAccountingSink};
pub use measure::{ByteSized, CanonicalKey, MeasuredValue};
