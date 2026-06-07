//! Resource-group fairness and admission control.
//!
//! The frozen [`ResourceGroupScheduler`] trait answers a single admission
//! question. [`WeightedResourceGroupScheduler`] is the concrete deterministic
//! implementation: it tracks per-group usage, enforces each group's hard cap,
//! and grants burst (borrowing from the shared pool) only up to a weighted
//! fair share so that one noisy owner cannot starve the others.

use std::collections::BTreeMap;

use crate::domain::policy::ResourceGroup;

/// Admission authority for resource groups.
pub trait ResourceGroupScheduler {
    /// Whether `group_name` may grow by `requested_bytes` right now.
    fn admits(&self, group_name: &str, requested_bytes: u64) -> bool;
}

/// Deterministic weighted-fair scheduler.
///
/// Admission rule for a group requesting `delta`:
/// 1. growth within the group's own reserved/hard cap is always admitted;
/// 2. growth above the hard cap may borrow from the shared burst pool, but only
///    up to that group's weighted fair share of the pool
///    (`admission_weight / sum(admission_weight)`);
/// 3. an unknown group is never admitted (fail-closed).
///
/// "Fair share" is recomputed from live usage on every call, so a group that
/// has already consumed its weighted slice is denied further burst even while
/// other groups still have headroom — this is what prevents starvation under a
/// repeatedly noisy owner.
#[derive(Clone, Debug, Default)]
pub struct WeightedResourceGroupScheduler {
    groups: BTreeMap<String, ResourceGroup>,
    usage: BTreeMap<String, u64>,
    shared_burst_pool_bytes: u64,
}

impl WeightedResourceGroupScheduler {
    #[must_use]
    pub fn new(groups: BTreeMap<String, ResourceGroup>, shared_burst_pool_bytes: u64) -> Self {
        Self { groups, usage: BTreeMap::new(), shared_burst_pool_bytes }
    }

    /// Record committed growth for a group so subsequent admission decisions see
    /// live usage. Returns the new usage total for that group.
    pub fn record_usage(&mut self, group_name: &str, delta: u64) -> u64 {
        let entry = self.usage.entry(group_name.to_string()).or_insert(0);
        *entry = entry.saturating_add(delta);
        *entry
    }

    /// Record a release for a group.
    pub fn record_release(&mut self, group_name: &str, delta: u64) -> u64 {
        let entry = self.usage.entry(group_name.to_string()).or_insert(0);
        *entry = entry.saturating_sub(delta);
        *entry
    }

    #[must_use]
    pub fn current_usage(&self, group_name: &str) -> u64 {
        self.usage.get(group_name).copied().unwrap_or(0)
    }

    fn total_weight(&self) -> u64 {
        self.groups.values().map(|g| u64::from(g.admission_weight)).sum()
    }

    /// Weighted fair share of the shared burst pool for one group.
    fn fair_burst_share(&self, group: &ResourceGroup) -> u64 {
        let total = self.total_weight();
        if total == 0 {
            return 0;
        }
        // floor division; deterministic for a fixed weight set.
        (self.shared_burst_pool_bytes / total).saturating_mul(u64::from(group.admission_weight))
    }
}

impl ResourceGroupScheduler for WeightedResourceGroupScheduler {
    fn admits(&self, group_name: &str, requested_bytes: u64) -> bool {
        let Some(group) = self.groups.get(group_name) else {
            return false;
        };
        let current = self.current_usage(group_name);
        let after = current.saturating_add(requested_bytes);

        if after <= group.hard_cap_bytes {
            return true;
        }

        // Above hard cap: only admit the overflow if it fits within this
        // group's weighted fair share of the shared burst pool.
        let borrow = after - group.hard_cap_bytes;
        borrow <= self.fair_burst_share(group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policy::ReclaimPriority;

    fn group(hard: u64, weight: u16) -> ResourceGroup {
        ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: hard / 2,
            hard_cap_bytes: hard,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: weight,
        }
    }

    fn two_group_scheduler() -> WeightedResourceGroupScheduler {
        let mut groups = BTreeMap::new();
        groups.insert("fg".to_string(), group(1_000, 3)); // weight 3
        groups.insert("bg".to_string(), group(1_000, 1)); // weight 1
                                                          // shared burst pool of 400; total weight 4 => share = (400/4)*weight.
        WeightedResourceGroupScheduler::new(groups, 400)
    }

    #[test]
    fn within_hard_cap_is_always_admitted() {
        let s = two_group_scheduler();
        assert!(s.admits("fg", 1_000), "request up to hard cap admitted");
        assert!(s.admits("bg", 500));
    }

    #[test]
    fn unknown_group_is_denied_fail_closed() {
        let s = two_group_scheduler();
        assert!(!s.admits("ghost", 1), "unknown group must never be admitted");
    }

    // --- Negative path 4: starvation under repeated noisy owner growth
    #[test]
    fn noisy_group_cannot_borrow_beyond_weighted_share() {
        let mut s = two_group_scheduler();
        // fg weight 3 => burst share = (400/4)*3 = 300.
        // bg weight 1 => burst share = (400/4)*1 = 100.

        // fg fills its hard cap, then keeps trying to grow (noisy owner).
        assert_eq!(s.record_usage("fg", 1_000), 1_000);
        // borrow up to 300 over hard cap is allowed.
        assert!(s.admits("fg", 300), "fg may borrow up to its 300 fair share");
        // but 301 over hard cap exceeds fg's share -> denied (no starvation of bg).
        assert!(!s.admits("fg", 301), "fg cannot exceed its weighted burst share");

        // Even after fg already borrowed its full share, bg still gets its own
        // share -> bg is not starved by fg's noise.
        assert_eq!(s.record_usage("fg", 300), 1_300); // fg consumed its share
        assert_eq!(s.record_usage("bg", 1_000), 1_000); // bg at its hard cap
        assert!(s.admits("bg", 100), "bg keeps its own 100 fair share despite fg noise");
        assert!(!s.admits("bg", 101), "bg still bounded by its own share");
    }

    #[test]
    fn release_returns_burst_capacity() {
        let mut s = two_group_scheduler();
        s.record_usage("fg", 1_300); // hard cap + full share
        assert!(!s.admits("fg", 1), "no more burst available");
        s.record_release("fg", 300); // give back the borrowed share
        assert!(s.admits("fg", 200), "released capacity becomes admissible again");
    }

    // scenario D-local: an empty scheduler (no groups) has total_weight == 0.
    // fair_burst_share must be 0 with no divide-by-zero, growth within an
    // (unknown) group's cap cannot be admitted, and unknown groups are denied.
    #[test]
    fn empty_scheduler_has_zero_share_and_denies_all() {
        let s = WeightedResourceGroupScheduler::new(BTreeMap::new(), 400);
        // No groups => total weight 0 => any group is unknown => denied.
        assert!(!s.admits("fg", 0), "unknown group denied even for a zero request");
        assert!(!s.admits("fg", 100), "unknown group denied (fail-closed, no panic)");
        // Burst share for a stray group definition is exactly 0 when total
        // weight is 0 (documents the no-divide-by-zero contract).
        assert_eq!(s.fair_burst_share(&group(1_000, 5)), 0, "zero total weight => zero share");
    }

    // scenario D-local: a single registered group still funds burst, but a group
    // name that is NOT registered is denied regardless of request size.
    #[test]
    fn unknown_group_denied_even_with_pool_available() {
        let mut groups = BTreeMap::new();
        groups.insert("fg".to_string(), group(1_000, 1));
        let s = WeightedResourceGroupScheduler::new(groups, 400);
        assert!(s.admits("fg", 1_000), "registered group admitted within cap");
        assert!(!s.admits("unregistered", 1), "unregistered group denied despite pool headroom");
    }

    // scenario D-local: floor-division of the pool leaves a remainder that is
    // deliberately NOT handed out — the sum of fair shares can be below the pool.
    // This documents the leftover-pool behavior so it is intentional, not a bug.
    #[test]
    fn floor_division_pool_remainder_is_left_unallocated() {
        let mut groups = BTreeMap::new();
        // total weight 3, pool 400 => 400/3 = 133 (floor), remainder 1 byte.
        groups.insert("a".to_string(), group(0, 1));
        groups.insert("b".to_string(), group(0, 1));
        groups.insert("c".to_string(), group(0, 1));
        let s = WeightedResourceGroupScheduler::new(groups, 400);
        let share = s.fair_burst_share(&group(0, 1));
        assert_eq!(share, 133, "floor(400/3)*1 == 133");
        let total_shares = share * 3;
        assert_eq!(total_shares, 399, "sum of floor shares is 399");
        assert!(
            total_shares < 400,
            "leftover pool remainder (1 byte) is intentionally unallocated, not over-committed"
        );
        // Each group may borrow exactly its floor share and not one byte more.
        assert!(s.admits("a", 133), "group may borrow its full floor share");
        assert!(!s.admits("a", 134), "group cannot reach into the unallocated remainder");
    }

    // scenario D-local: record_usage then record_release is a clean round-trip
    // that re-admits a previously denied burst, and never underflows past zero.
    #[test]
    fn usage_release_round_trip_readmits_burst() {
        let mut s = two_group_scheduler(); // fg share 300 over hard cap 1_000
        assert_eq!(s.current_usage("fg"), 0);
        // Consume hard cap + full burst share.
        assert_eq!(s.record_usage("fg", 1_300), 1_300);
        assert!(!s.admits("fg", 1), "fully bursted: no further admission");
        // Release the full burst; usage returns to exactly the hard cap.
        assert_eq!(s.record_release("fg", 300), 1_000);
        assert_eq!(s.current_usage("fg"), 1_000, "round-trip restores exact prior usage");
        // The freed burst is admissible again.
        assert!(s.admits("fg", 300), "released burst capacity is re-admitted");
        // Saturating release never underflows below zero.
        assert_eq!(s.record_release("fg", u64::MAX), 0, "over-release saturates at zero");
        assert_eq!(s.current_usage("fg"), 0);
    }

    #[test]
    fn huge_pool_and_weights_compute_a_non_wrapping_fair_share() {
        let mut groups = BTreeMap::new();
        groups.insert("heavy".to_string(), group(0, u16::MAX));
        groups.insert("light".to_string(), group(0, 1));
        let s = WeightedResourceGroupScheduler::new(groups, u64::MAX);
        let heavy = group(0, u16::MAX);
        let share = s.fair_burst_share(&heavy);
        let expected = (u64::MAX / (u64::from(u16::MAX) + 1)).saturating_mul(u64::from(u16::MAX));
        assert_eq!(share, expected, "fair share math must stay exact under huge inputs");
        assert!(s.admits("heavy", share), "group may borrow exactly its computed fair share");
        assert!(
            !s.admits("heavy", share.saturating_add(1)),
            "group may not borrow one byte beyond its computed fair share"
        );
    }

    #[test]
    fn repeated_saturating_usage_release_round_trip_never_wraps() {
        let mut s = two_group_scheduler();
        for _ in 0..8 {
            assert_eq!(s.record_usage("fg", u64::MAX), u64::MAX);
            assert_eq!(s.current_usage("fg"), u64::MAX);
            assert_eq!(s.record_release("fg", u64::MAX), 0);
            assert_eq!(s.current_usage("fg"), 0);
        }
    }
}
