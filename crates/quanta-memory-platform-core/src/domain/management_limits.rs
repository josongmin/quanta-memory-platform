//! Independent bounds for memory-platform bookkeeping.
//!
//! These limits are deliberately separate from byte budgets: a zero-byte
//! reservation still creates a live single-use authority and owner/group
//! indexes, so byte accounting alone cannot bound management metadata.

/// Maximum number of simultaneously live reservation tickets owned by one broker.
///
/// This cap bounds ticket state and the mirrored owner/group/deferred-cleanup
/// indexes, including when every reservation accounts for zero bytes. It is a
/// logical cardinality bound, not a promise about process RSS or allocator cost.
pub const MAX_LIVE_TICKETS_PER_BROKER: usize = 16_384;

/// Maximum UTF-8 byte length of an owner identifier retained by the ledger.
///
/// Bounding each key prevents a bounded ticket count from multiplying into an
/// unbounded per-entry string payload. UTF-8 bytes are used because that is the
/// actual storage cost of Rust `String`, rather than a character-count proxy.
pub const MAX_OWNER_KEY_BYTES: usize = 256;

/// Maximum UTF-8 byte length of a resource-group identifier retained by policy
/// or copied into per-ticket release bookkeeping.
pub const MAX_RESOURCE_GROUP_KEY_BYTES: usize = 256;

/// Maximum deferred authorities processed while holding the broker lock in one
/// cleanup batch. The queue itself remains bounded by the live-ticket limit.
pub const MAX_DEFERRED_TICKET_DRAIN_BATCH: usize = 1_024;
