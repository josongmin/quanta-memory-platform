//! Experimental concurrent size index over `scc` (opt-in
//! `experimental-revocable-scc-index` feature).
//!
//! This is not a canonical revocable-store backing: it owns neither payloads
//! nor accounting transfer authority.
//!
//! A concurrent `id -> charged_bytes` index for revocable entries. Unlike the
//! previous stub, `insert` no longer silently swallows its result: a duplicate
//! key is reported, never dropped on the floor.

/// An `scc`-backed concurrent revocable index (`id -> charged_bytes`).
#[derive(Debug, Default)]
pub struct SccBacking {
    index: scc::HashMap<String, usize>,
}

impl SccBacking {
    /// Record an entry's charged byte size. Returns `true` if newly inserted,
    /// `false` if `key` already existed (the prior value is left untouched). The
    /// result is surfaced, never swallowed.
    pub fn insert(&self, key: String, charged_bytes: usize) -> bool {
        self.index.insert_sync(key, charged_bytes).is_ok()
    }

    /// The charged byte size recorded for `key`, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<usize> {
        self.index.read_sync(key, |_, value| *value)
    }

    /// Remove an entry, returning its recorded byte size if it was present.
    pub fn remove(&self, key: &str) -> Option<usize> {
        self.index.remove_sync(key).map(|(_, value)| value)
    }

    /// Number of resident entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_reports_new_vs_duplicate_and_does_not_swallow() {
        let backing = SccBacking::default();
        assert!(backing.insert("k".to_string(), 10), "first insert is new");
        // A duplicate key is REPORTED (false), not silently dropped.
        assert!(!backing.insert("k".to_string(), 20), "duplicate key insert returns false");
        // The original value is preserved (insert did not overwrite).
        assert_eq!(backing.get("k"), Some(10));
        assert_eq!(backing.len(), 1);
    }

    #[test]
    fn remove_returns_recorded_size_and_empties() {
        let backing = SccBacking::default();
        backing.insert("a".to_string(), 7);
        assert!(!backing.is_empty());
        assert_eq!(backing.remove("a"), Some(7));
        assert_eq!(backing.remove("a"), None, "second remove finds nothing");
        assert!(backing.is_empty());
    }
}
