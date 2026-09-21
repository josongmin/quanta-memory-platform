//! Experimental size index over `moka` (opt-in
//! `experimental-revocable-moka-index` feature).
//!
//! This is not a canonical revocable-store backing: it owns neither payloads
//! nor accounting transfer authority, and autonomous eviction has no receipt.
//!
//! A size-bounded concurrent index keyed by entry id, holding each entry's
//! charged byte size. Eviction is `moka`'s own capacity policy, so callers must
//! treat entries as reclaimable at any time — which is exactly the revocable
//! contract.

/// A `moka`-backed concurrent revocable index (`id -> charged_bytes`).
#[derive(Clone, Debug)]
pub struct MokaBacking {
    cache: moka::sync::Cache<String, usize>,
}

impl MokaBacking {
    /// Create with an explicit maximum entry capacity. Beyond it, `moka` evicts
    /// under its own size policy — entries are reclaimable, never pinned here.
    #[must_use]
    pub fn with_capacity(max_entries: u64) -> Self {
        Self { cache: moka::sync::Cache::new(max_entries) }
    }

    /// Record an entry's charged byte size under `key`.
    pub fn insert(&self, key: String, charged_bytes: usize) {
        self.cache.insert(key, charged_bytes);
    }

    /// The charged byte size recorded for `key`, if still resident.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<usize> {
        self.cache.get(key)
    }

    /// Approximate number of resident entries (after pending maintenance).
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.cache.run_pending_tasks();
        self.cache.entry_count()
    }
}

impl Default for MokaBacking {
    /// A modest default capacity; prefer [`Self::with_capacity`] to size it for
    /// the deployment rather than relying on this constant.
    fn default() -> Self {
        Self::with_capacity(128)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_get_returns_the_recorded_size() {
        let backing = MokaBacking::with_capacity(16);
        backing.insert("entry-1".to_string(), 4_096);
        assert_eq!(backing.get("entry-1"), Some(4_096));
        assert_eq!(backing.get("missing"), None);
    }

    #[test]
    fn entry_count_reflects_inserts_after_maintenance() {
        let backing = MokaBacking::with_capacity(16);
        backing.insert("a".to_string(), 1);
        backing.insert("b".to_string(), 2);
        assert_eq!(backing.entry_count(), 2);
    }

    #[test]
    fn default_is_usable() {
        let backing = MokaBacking::default();
        backing.insert("x".to_string(), 9);
        assert_eq!(backing.get("x"), Some(9));
    }

    // The backing exists to bound residency: inserting well beyond capacity must
    // evict (entries are reclaimable), so the resident count stays near the cap
    // rather than growing unbounded. A broken capacity (e.g. unbounded) would fail
    // this — the whole point of the moka backing.
    #[test]
    fn inserting_beyond_capacity_evicts_down_to_the_bound() {
        let cap = 4u64;
        let backing = MokaBacking::with_capacity(cap);
        for i in 0..200u32 {
            backing.insert(format!("k{i}"), i as usize);
        }
        let resident = backing.entry_count();
        assert!(
            resident <= cap,
            "moka must evict beyond its capacity {cap}, but {resident} entries are resident"
        );
        // And the cache still works: a fresh insert is retrievable.
        backing.insert("fresh".to_string(), 7);
        assert_eq!(
            backing.get("fresh"),
            Some(7),
            "the bounded cache remains usable after eviction"
        );
    }
}
