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

/// A value that can report the number of bytes it will charge when retained.
///
/// Blanket impls cover the common portable byte carriers. Callers with a custom
/// payload type implement this once and every facade store charges it correctly.
pub trait ByteSized {
    /// Number of bytes this value charges against a store's accounting axis.
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
