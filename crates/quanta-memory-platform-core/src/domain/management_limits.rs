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

/// Maximum memory classes declared by one residency policy. The class axis is
/// a closed enum (well below this cap), so the bound is structural: it also
/// bounds reclaim-candidate vectors and per-class postimage scans.
pub const MAX_POLICY_CLASSES_PER_POLICY: usize = 32;

/// Maximum resource groups declared by one residency policy.
///
/// Group-scale work runs while the broker lock is held: every recharge clones
/// the whole scheduler and every soft-threshold crossing or reclaim denial
/// clones and sorts each group name. This cap bounds that lock-held work and
/// the mirrored scheduler maps to trusted-configuration scale (presets declare
/// a handful of groups). It is a logical bookkeeping bound, not a latency or
/// RSS promise.
pub const MAX_RESOURCE_GROUPS_PER_POLICY: usize = 256;

/// Maximum owner-to-group mappings declared by one residency policy. Same
/// trusted-configuration bound as [`MAX_RESOURCE_GROUPS_PER_POLICY`]: each
/// mapping is mirrored into per-ticket release bookkeeping and rescanned by
/// remap guards and policy installation under the lock.
pub const MAX_OWNER_GROUP_MAPPINGS_PER_POLICY: usize = 4_096;

/// Maximum per-owner quotas declared by one residency policy. Quotas are
/// inert without live tickets, so this bound is calibrated at a quarter of
/// [`MAX_LIVE_TICKETS_PER_BROKER`]; it bounds the reconfigure postimage scan
/// and resolve-time validation.
pub const MAX_OWNER_QUOTAS_PER_POLICY: usize = 4_096;
