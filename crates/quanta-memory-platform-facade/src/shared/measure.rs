//! Domain-neutral byte-measurement glue shared across portable store slices.
//!
//! Every store slice needs to estimate "how many bytes does this value charge"
//! before it reserves capacity. That estimate must come from the *caller's* type,
//! not from any host runtime noun. [`ByteSized`] is the single shared contract for
//! that estimate so payload / retained-map / transcript slices all charge bytes
//! through one definition instead of each re-deriving an ad-hoc `len()`.
//!
//! This is shared glue (used by more than one slice) and therefore lives under
//! `shared/*`, never inside a slice or a backing crate.

use quanta_memory_platform_core::{MemoryError, MemoryResult};

/// Versioned portable *semantic* retention costs. These constants are not
/// `size_of` claims, allocator capacity, transient copies, or RSS measurements.
/// Changing a value changes the public admission policy and requires a new
/// formula version rather than silently rewriting historical receipts.
pub mod semantic_cost_v1 {
    /// Per retained entry's logical state and lease ownership.
    pub const ENTRY_STATE_BYTES: u64 = 16;
    /// A retained hash key's linkage and lookup slot.
    pub const HASH_LINK_BYTES: u64 = 24;
    /// One retained ordered-index node/lookup slot.
    pub const ORDERED_INDEX_BYTES: u64 = 24;
    /// One retained FIFO position and lookup slot.
    pub const FIFO_INDEX_BYTES: u64 = 16;
    /// A retained BLAKE3 digest or derived token.
    pub const DIGEST_BYTES: u64 = 32;
}

/// Checked cost of bytes retained by one store-owned lifetime component.
///
/// `total()` is the only conversion to a broker charge. A store must persist
/// that exact total with its non-Clone lease, then use it for stats and reclaim
/// instead of remeasuring mutable or caller-owned data on the release path.
/// A value's [`ByteSized`] implementation remains caller-reported: this model
/// does not claim to meter arbitrary heap allocations or allocator overhead.
///
/// These components *calculate* one charge's total; the core
/// `ChargeKind` enum separately *classifies* which owned lifetime component
/// that whole total belongs to. A kind bucket therefore includes the key,
/// index, and fixed overhead carried by the same lease and is not a promise
/// of per-field byte-accurate payload/metadata telemetry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetainedCostV1 {
    pub payload_bytes: u64,
    pub key_identity_bytes: u64,
    pub index_bytes: u64,
    pub tombstone_bytes: u64,
    pub fixed_overhead_bytes: u64,
}

impl RetainedCostV1 {
    /// Sum every component without saturating or wrapping a budget authority.
    pub fn total(self) -> MemoryResult<u64> {
        [
            self.payload_bytes,
            self.key_identity_bytes,
            self.index_bytes,
            self.tombstone_bytes,
            self.fixed_overhead_bytes,
        ]
        .into_iter()
        .try_fold(0u64, |total, component| {
            total
                .checked_add(component)
                .ok_or_else(|| MemoryError::capacity("retained-cost-v1 component sum overflow"))
        })
    }

    /// Combine independent retained lifetime components, checking both every
    /// field and the aggregate sum before returning a usable postimage.
    pub fn checked_add(self, other: Self) -> MemoryResult<Self> {
        let add = |a: u64, b: u64| {
            a.checked_add(b)
                .ok_or_else(|| MemoryError::capacity("retained-cost-v1 component overflow"))
        };
        let combined = Self {
            payload_bytes: add(self.payload_bytes, other.payload_bytes)?,
            key_identity_bytes: add(self.key_identity_bytes, other.key_identity_bytes)?,
            index_bytes: add(self.index_bytes, other.index_bytes)?,
            tombstone_bytes: add(self.tombstone_bytes, other.tombstone_bytes)?,
            fixed_overhead_bytes: add(self.fixed_overhead_bytes, other.fixed_overhead_bytes)?,
        };
        combined.total()?;
        Ok(combined)
    }
}

/// A value that reports its semantic byte charge when retained.
///
/// The returned number is captured at admission; stores do not poll it or
/// automatically adjust a live charge. Implementations for values with
/// interior-mutability/shared backing must either report a conservative upper
/// bound for every state reachable while retained, or require callers to use an
/// explicit recharge/replacement operation when retained cost changes. This is
/// a caller contract, not a physical allocation or RSS measurement.
///
/// Blanket impls cover the common portable byte carriers. Callers with a custom
/// payload type implement this once and every facade store charges it correctly.
pub trait ByteSized {
    /// Number of semantic bytes to charge at admission.
    ///
    /// Keep this estimate valid until release, or conservatively bound
    /// subsequent interior-mutable states as described on [`ByteSized`].
    fn byte_size(&self) -> u64;
}

impl ByteSized for Vec<u8> {
    fn byte_size(&self) -> u64 {
        self.len() as u64
    }
}

impl ByteSized for String {
    fn byte_size(&self) -> u64 {
        self.len() as u64
    }
}

impl ByteSized for &str {
    fn byte_size(&self) -> u64 {
        self.len() as u64
    }
}

impl ByteSized for [u8] {
    fn byte_size(&self) -> u64 {
        self.len() as u64
    }
}

impl<T: ByteSized + ?Sized> ByteSized for &T {
    fn byte_size(&self) -> u64 {
        (**self).byte_size()
    }
}

impl<T: ByteSized> ByteSized for Box<T> {
    fn byte_size(&self) -> u64 {
        (**self).byte_size()
    }
}

impl<T: ByteSized + ?Sized> ByteSized for std::sync::Arc<T> {
    fn byte_size(&self) -> u64 {
        (**self).byte_size()
    }
}

/// An opaque in-process value paired with a caller-supplied byte-size estimate.
///
/// Some callers need to retain a value whose type does not (and cannot) implement
/// [`ByteSized`] itself — most notably an erased in-process handle such as
/// `Arc<dyn Any + Send + Sync>`, where the orphan rule blocks the caller from
/// adding a `ByteSized` impl for that foreign type. [`MeasuredValue`] wraps any
/// `Clone` value with an explicit size the caller computes once, so it can be
/// stored in a retained-map store and accounted by the broker like any other
/// sized value.
///
/// This stays domain-neutral: it carries a `value` and a `size`, never a host
/// runtime noun. The size is the caller's responsibility and is charged verbatim;
/// the wrapper never re-derives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeasuredValue<T> {
    value: T,
    size: u64,
}

impl<T> MeasuredValue<T> {
    /// Pair a value with the number of bytes it charges when retained.
    pub fn new(value: T, size: u64) -> Self {
        Self { value, size }
    }

    /// The wrapped value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// The caller-supplied byte size charged for this value.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Consume the wrapper, returning the inner value.
    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> ByteSized for MeasuredValue<T> {
    fn byte_size(&self) -> u64 {
        self.size
    }
}

/// A value whose *canonical identity* can be derived, so a payload store can tell
/// whether the canonical payload already exists without charging it twice.
///
/// Identity is a portable byte key, never a host runtime reference type.
pub trait CanonicalKey {
    /// Stable identity bytes for canonical dedup.
    fn canonical_key(&self) -> Vec<u8>;
}

impl CanonicalKey for Vec<u8> {
    fn canonical_key(&self) -> Vec<u8> {
        self.clone()
    }
}

impl CanonicalKey for String {
    fn canonical_key(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::sync::Arc;

    #[test]
    fn retained_cost_v1_sums_each_component_exactly_once() {
        let cost = RetainedCostV1 {
            payload_bytes: 1,
            key_identity_bytes: 2,
            index_bytes: 3,
            tombstone_bytes: 4,
            fixed_overhead_bytes: 5,
        };
        assert_eq!(cost.total().expect("bounded cost"), 15);
        assert_eq!(cost.checked_add(cost).expect("combined cost").total().expect("total"), 30);
    }

    #[test]
    fn retained_cost_v1_rejects_component_and_cross_component_overflow() {
        let max_payload = RetainedCostV1 { payload_bytes: u64::MAX, ..Default::default() };
        assert!(max_payload.checked_add(max_payload).is_err(), "component overflow");
        assert!(
            max_payload
                .checked_add(RetainedCostV1 { key_identity_bytes: 1, ..Default::default() })
                .is_err(),
            "individual fields fit, but the aggregate charge does not"
        );
        assert!(
            RetainedCostV1 { payload_bytes: u64::MAX, index_bytes: 1, ..Default::default() }
                .total()
                .is_err(),
            "a direct struct literal must not bypass checked total"
        );
    }

    #[test]
    fn measured_value_charges_caller_supplied_size_not_inner_len() {
        // The wrapped value is a 3-byte vec, but the caller declares 4096 bytes
        // (e.g. a heap handle whose true footprint the caller knows).
        let measured = MeasuredValue::new(vec![1u8, 2, 3], 4096);
        assert_eq!(measured.byte_size(), 4096, "byte_size must echo the caller size");
        assert_eq!(measured.size(), 4096);
        assert_eq!(measured.value(), &vec![1u8, 2, 3]);
    }

    #[test]
    fn measured_value_into_inner_returns_wrapped_value() {
        let measured = MeasuredValue::new("payload".to_string(), 7);
        assert_eq!(measured.into_inner(), "payload".to_string());
    }

    #[test]
    fn measured_value_zero_size_is_charged_verbatim() {
        // NEGATIVE-PAIR: a declared zero size is charged as zero, never inferred
        // from the inner value's length.
        let measured = MeasuredValue::new(vec![0u8; 100], 0);
        assert_eq!(measured.byte_size(), 0, "zero declared size must not infer 100");
    }

    #[test]
    fn arc_byte_size_delegates_to_inner() {
        let arc: Arc<Vec<u8>> = Arc::new(vec![0u8; 12]);
        assert_eq!(arc.byte_size(), 12, "Arc<T> charges the inner T's byte_size");
    }

    #[test]
    fn cloning_measured_arc_shares_the_allocation_not_a_deep_copy() {
        // scenario F (dedup-hit no re-clone): a MeasuredValue<Arc<_>> clones only
        // the Arc handle (refcount bump), never deep-copying the payload — the
        // dedup-hit path must reuse the same allocation. Proven via Arc::ptr_eq.
        let payload: Arc<Vec<u8>> = Arc::new(vec![9u8; 1024]);
        let measured = MeasuredValue::new(Arc::clone(&payload), 1024);

        let cloned = measured.clone();
        assert!(
            Arc::ptr_eq(measured.value(), cloned.value()),
            "clone must share the same Arc allocation, not deep-copy the payload"
        );
        // The strong count rose by the number of live Arc handles, confirming no
        // independent payload copy was made.
        assert_eq!(Arc::strong_count(&payload), 3, "payload + measured + cloned share one alloc");
    }

    #[test]
    fn cloning_measured_value_preserves_size_and_value_context() {
        // scenario F (context preserved): cloning a MeasuredValue must carry BOTH
        // the value and its caller-supplied size ("context") verbatim — the dedup
        // path never recomputes or drops the size.
        let measured = MeasuredValue::new("ctx-payload".to_string(), 8192);
        let cloned = measured.clone();
        assert_eq!(cloned.size(), 8192, "caller size context preserved across clone");
        assert_eq!(cloned.value(), &"ctx-payload".to_string(), "value preserved across clone");
        // Equality holds across the full (value, size) tuple.
        assert_eq!(measured, cloned);
    }

    #[test]
    fn measured_arc_any_handle_charges_caller_size() {
        // The opaque-handle substrate: Arc<dyn Any> has no ByteSized impl of its
        // own, so MeasuredValue supplies the size accounting.
        let handle: Arc<dyn Any + Send + Sync> = Arc::new(42u64);
        let measured = MeasuredValue::new(handle, 2048);
        assert_eq!(measured.byte_size(), 2048);
    }
}
