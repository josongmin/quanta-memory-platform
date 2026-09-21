//! Resource-group fairness and admission control.
//!
//! The frozen [`ResourceGroupScheduler`] trait answers a single admission
//! question. [`WeightedResourceGroupScheduler`] is the concrete deterministic
//! implementation: it tracks per-group usage, enforces each group's hard cap,
//! and grants burst (borrowing from the shared pool) only up to a weighted
//! fair share so that one noisy owner cannot starve the others.

use std::collections::BTreeMap;

use crate::domain::errors::{MemoryError, MemoryResult};
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
/// Weighted shares are policy-static and are precomputed once when the
/// scheduler is built. Admission combines that cached allowance with live
/// usage, so a group that has already consumed its weighted slice is denied
/// further burst even while other groups still have headroom. Reconfiguration
/// builds a new scheduler and therefore a new allocation table.
#[derive(Clone, Debug, Default)]
pub struct WeightedResourceGroupScheduler {
    groups: BTreeMap<String, ResourceGroup>,
    usage: BTreeMap<String, u64>,
    shared_burst_pool_bytes: u64,
    /// Hamilton allocation of the shared pool. Keeping this policy-static
    /// result avoids an O(group_count * log(group_count)) sort on every
    /// admission decision.
    fair_burst_shares: BTreeMap<String, u64>,
    /// Sum of `max(group.reserved_bytes - live_usage, 0)`. Kept as u128 so a
    /// large group registry cannot overflow while the externally enforced
    /// non-headroom band remains u64.
    unused_reserved_floor_bytes: u128,
}

impl WeightedResourceGroupScheduler {
    #[must_use]
    pub fn new(groups: BTreeMap<String, ResourceGroup>, shared_burst_pool_bytes: u64) -> Self {
        let unused_reserved_floor_bytes =
            groups.values().fold(0u128, |total, group| total + u128::from(group.reserved_bytes));
        let fair_burst_shares =
            Self::precompute_fair_burst_shares(&groups, shared_burst_pool_bytes);
        Self {
            groups,
            usage: BTreeMap::new(),
            shared_burst_pool_bytes,
            fair_burst_shares,
            unused_reserved_floor_bytes,
        }
    }

    fn unused_floor_for(&self, group_name: &str, usage: u64) -> u64 {
        self.groups.get(group_name).map_or(0, |group| group.reserved_bytes.saturating_sub(usage))
    }

    /// Checked growth postimage without mutating scheduler state.
    pub(crate) fn usage_after_growth(&self, group_name: &str, delta: u64) -> MemoryResult<u64> {
        if !self.groups.contains_key(group_name) {
            return Err(MemoryError::InvalidState(format!(
                "cannot record usage for unknown resource group {group_name}"
            )));
        }
        self.current_usage(group_name)
            .checked_add(delta)
            .ok_or_else(|| MemoryError::capacity("resource-group usage exceeds u64"))
    }

    /// Checked release postimage without mutating scheduler state.
    pub(crate) fn usage_after_release(&self, group_name: &str, delta: u64) -> MemoryResult<u64> {
        if !self.groups.contains_key(group_name) {
            return Err(MemoryError::InvalidState(format!(
                "cannot record release for unknown resource group {group_name}"
            )));
        }
        self.current_usage(group_name)
            .checked_sub(delta)
            .ok_or_else(|| MemoryError::InvalidState("resource-group release exceeds usage".into()))
    }

    /// Record committed growth for a group so subsequent admission decisions see
    /// live usage. Returns the new usage total for that group.
    pub fn record_usage(&mut self, group_name: &str, delta: u64) -> MemoryResult<u64> {
        let before = self.current_usage(group_name);
        let after = self.usage_after_growth(group_name, delta)?;
        let unused_before = self.unused_floor_for(group_name, before);
        let unused_after = self.unused_floor_for(group_name, after);
        self.unused_reserved_floor_bytes = self
            .unused_reserved_floor_bytes
            .checked_sub(u128::from(unused_before))
            .and_then(|total| total.checked_add(u128::from(unused_after)))
            .expect("scheduler floor aggregate mirrors registered groups");
        self.usage.insert(group_name.to_string(), after);
        Ok(after)
    }

    /// Record a release for a group.
    pub fn record_release(&mut self, group_name: &str, delta: u64) -> MemoryResult<u64> {
        let before = self.current_usage(group_name);
        let after = self.usage_after_release(group_name, delta)?;
        let unused_before = self.unused_floor_for(group_name, before);
        let unused_after = self.unused_floor_for(group_name, after);
        self.unused_reserved_floor_bytes = self
            .unused_reserved_floor_bytes
            .checked_sub(u128::from(unused_before))
            .and_then(|total| total.checked_add(u128::from(unused_after)))
            .expect("scheduler floor aggregate mirrors registered groups");
        self.usage.insert(group_name.to_string(), after);
        Ok(after)
    }

    #[must_use]
    pub fn current_usage(&self, group_name: &str) -> u64 {
        self.usage.get(group_name).copied().unwrap_or(0)
    }

    /// Validate the complete resource-group reservation postimage against the
    /// non-headroom band. Every group's unused floor remains protected, while
    /// bytes already consumed by that group reduce its remaining floor.
    pub(crate) fn protected_floor_fits(
        &self,
        non_headroom_usage: u64,
        non_headroom_band: u64,
    ) -> MemoryResult<()> {
        self.protected_floor_fits_with_unused(
            non_headroom_usage,
            non_headroom_band,
            self.unused_reserved_floor_bytes,
        )
    }

    /// Validate the floor after one prospective group growth without cloning
    /// the scheduler or scanning every configured group.
    pub(crate) fn protected_floor_fits_after_growth(
        &self,
        group_name: Option<&str>,
        delta: u64,
        non_headroom_usage: u64,
        non_headroom_band: u64,
    ) -> MemoryResult<()> {
        let prospective_unused = if let Some(group_name) = group_name {
            let before = self.current_usage(group_name);
            let after = self.usage_after_growth(group_name, delta)?;
            self.unused_reserved_floor_bytes
                .checked_sub(u128::from(self.unused_floor_for(group_name, before)))
                .and_then(|total| {
                    total.checked_add(u128::from(self.unused_floor_for(group_name, after)))
                })
                .ok_or_else(|| {
                    MemoryError::InvalidState(
                        "resource-group floor aggregate is inconsistent".into(),
                    )
                })?
        } else {
            self.unused_reserved_floor_bytes
        };
        self.protected_floor_fits_with_unused(
            non_headroom_usage,
            non_headroom_band,
            prospective_unused,
        )
    }

    fn protected_floor_fits_with_unused(
        &self,
        non_headroom_usage: u64,
        non_headroom_band: u64,
        unused_floor: u128,
    ) -> MemoryResult<()> {
        let protected_postimage =
            u128::from(non_headroom_usage).checked_add(unused_floor).ok_or_else(|| {
                MemoryError::capacity("resource-group protected postimage exceeds u128")
            })?;
        if protected_postimage > u128::from(non_headroom_band) {
            return Err(MemoryError::capacity(format!(
                "resource-group usage {non_headroom_usage} plus unused floors {unused_floor} \
                 exceeds non-headroom band {non_headroom_band}"
            )));
        }
        Ok(())
    }

    /// Precompute Hamilton's largest-remainder allocation. Every group receives
    /// the floor of its exact weighted share, then leftover bytes are assigned
    /// by descending fractional remainder with the stable group name as the
    /// tie break. The sum is exactly the configured pool when total weight is
    /// non-zero, including huge inputs.
    fn precompute_fair_burst_shares(
        groups: &BTreeMap<String, ResourceGroup>,
        shared_burst_pool_bytes: u64,
    ) -> BTreeMap<String, u64> {
        let total: u128 = groups.values().map(|group| u128::from(group.admission_weight)).sum();
        if total == 0 {
            return groups.keys().map(|name| (name.clone(), 0)).collect();
        }
        let pool = u128::from(shared_burst_pool_bytes);
        let mut shares = BTreeMap::new();
        let mut allocated_base = 0u128;
        let mut remainders: Vec<(&str, u128)> = groups
            .iter()
            .map(|(name, candidate)| {
                let numerator = pool * u128::from(candidate.admission_weight);
                let base = numerator / total;
                allocated_base += base;
                shares.insert(
                    name.clone(),
                    u64::try_from(base).expect("a weighted share cannot exceed the u64 pool"),
                );
                (name.as_str(), numerator % total)
            })
            .collect();
        remainders.sort_by(|(left_name, left_remainder), (right_name, right_remainder)| {
            right_remainder.cmp(left_remainder).then_with(|| left_name.cmp(right_name))
        });
        let leftover = pool - allocated_base;
        for (index, (name, _)) in remainders.into_iter().enumerate() {
            if u128::try_from(index).expect("usize always fits u128") >= leftover {
                break;
            }
            if let Some(share) = shares.get_mut(name) {
                *share += 1;
            }
        }
        shares
    }

    /// Cached weighted fair share of the shared burst pool for one named group.
    fn fair_burst_share(&self, group_name: &str) -> u64 {
        self.fair_burst_shares.get(group_name).copied().unwrap_or(0)
    }

    fn burst_allowance(&self, group_name: &str, group: &ResourceGroup) -> u64 {
        let percentage_cap = u64::try_from(
            u128::from(self.shared_burst_pool_bytes) * u128::from(group.burst_percent) / 100,
        )
        .unwrap_or(u64::MAX);
        self.fair_burst_share(group_name).min(percentage_cap)
    }
}

impl ResourceGroupScheduler for WeightedResourceGroupScheduler {
    fn admits(&self, group_name: &str, requested_bytes: u64) -> bool {
        let Some(group) = self.groups.get(group_name) else {
            return false;
        };
        let current = self.current_usage(group_name);
        let Some(after) = current.checked_add(requested_bytes) else {
            return false;
        };

        if after <= group.hard_cap_bytes {
            return true;
        }

        // Above hard cap: only admit the overflow if it fits within this
        // group's weighted fair share of the shared burst pool.
        let borrow = after - group.hard_cap_bytes;
        borrow <= self.burst_allowance(group_name, group)
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
            burst_percent: 100,
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
        let mut s = two_group_scheduler();
        assert!(!s.admits("ghost", 1), "unknown group must never be admitted");
        assert!(matches!(s.record_usage("ghost", 0), Err(MemoryError::InvalidState(_))));
        assert!(matches!(s.record_release("ghost", 0), Err(MemoryError::InvalidState(_))));
        assert_eq!(s.usage.len(), 0, "unknown groups must not grow scheduler state");
    }

    // --- Negative path 4: starvation under repeated noisy owner growth
    #[test]
    fn noisy_group_cannot_borrow_beyond_weighted_share() {
        let mut s = two_group_scheduler();
        // fg weight 3 => burst share = (400/4)*3 = 300.
        // bg weight 1 => burst share = (400/4)*1 = 100.

        // fg fills its hard cap, then keeps trying to grow (noisy owner).
        assert_eq!(s.record_usage("fg", 1_000).expect("record"), 1_000);
        // borrow up to 300 over hard cap is allowed.
        assert!(s.admits("fg", 300), "fg may borrow up to its 300 fair share");
        // but 301 over hard cap exceeds fg's share -> denied (no starvation of bg).
        assert!(!s.admits("fg", 301), "fg cannot exceed its weighted burst share");

        // Even after fg already borrowed its full share, bg still gets its own
        // share -> bg is not starved by fg's noise.
        assert_eq!(s.record_usage("fg", 300).expect("record"), 1_300); // fg consumed its share
        assert_eq!(s.record_usage("bg", 1_000).expect("record"), 1_000); // bg at its hard cap
        assert!(s.admits("bg", 100), "bg keeps its own 100 fair share despite fg noise");
        assert!(!s.admits("bg", 101), "bg still bounded by its own share");
    }

    #[test]
    fn release_returns_burst_capacity() {
        let mut s = two_group_scheduler();
        s.record_usage("fg", 1_300).expect("record"); // hard cap + full share
        assert!(!s.admits("fg", 1), "no more burst available");
        s.record_release("fg", 300).expect("release"); // give back the borrowed share
        assert!(s.admits("fg", 200), "released capacity becomes admissible again");
    }

    #[test]
    fn prospective_floor_aggregate_matches_committed_growth_and_release() {
        let mut groups = BTreeMap::new();
        let mut a = group(1_000, 1);
        a.reserved_bytes = 100;
        let mut b = group(1_000, 1);
        b.reserved_bytes = 200;
        groups.insert("a".to_string(), a);
        groups.insert("b".to_string(), b);
        let mut scheduler = WeightedResourceGroupScheduler::new(groups, 0);

        scheduler
            .protected_floor_fits_after_growth(Some("a"), 100, 800, 1_000)
            .expect("prospective growth consumes a's floor exactly");
        assert!(matches!(
            scheduler.protected_floor_fits_after_growth(Some("a"), 100, 801, 1_000),
            Err(MemoryError::Capacity(_))
        ));

        scheduler.record_usage("a", 100).expect("commit growth");
        scheduler.protected_floor_fits(800, 1_000).expect("committed aggregate matches preview");
        scheduler.record_release("a", 100).expect("release growth");
        scheduler.protected_floor_fits(700, 1_000).expect("release restores reserved floor");
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
        assert_eq!(s.fair_burst_share("fg"), 0, "unknown group => zero share");
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

    // The complete shared pool is assigned deterministically. Integer division
    // may leave a remainder, but it must not become unreachable capacity.
    #[test]
    fn weighted_share_distributes_remainder_work_conservingly() {
        let mut groups = BTreeMap::new();
        // total weight 3, pool 400 => 400/3 = 133 (floor), remainder 1 byte.
        groups.insert("a".to_string(), group(0, 1));
        groups.insert("b".to_string(), group(0, 1));
        groups.insert("c".to_string(), group(0, 1));
        let s = WeightedResourceGroupScheduler::new(groups, 400);
        let a = s.fair_burst_share("a");
        let b = s.fair_burst_share("b");
        let c = s.fair_burst_share("c");
        assert_eq!(a + b + c, 400, "all shared bytes must have a deterministic owner");
        assert_eq!((a, b, c), (134, 133, 133), "name order breaks equal remainders");
        assert!(s.admits("a", 134));
        assert!(!s.admits("a", 135));
    }

    #[test]
    fn burst_percent_changes_admission_outcome() {
        let mut groups = BTreeMap::new();
        let mut capped = group(1_000, 1);
        capped.burst_percent = 10;
        groups.insert("capped".to_string(), capped);
        let s = WeightedResourceGroupScheduler::new(groups, 1_000);

        assert!(s.admits("capped", 1_100), "10% permits 100 bytes above hard");
        assert!(!s.admits("capped", 1_101), "burst percent caps weighted pool access");
    }

    // scenario D-local: record_usage then record_release is a clean round-trip
    // that re-admits a previously denied burst, and never underflows past zero.
    #[test]
    fn usage_release_round_trip_readmits_burst() {
        let mut s = two_group_scheduler(); // fg share 300 over hard cap 1_000
        assert_eq!(s.current_usage("fg"), 0);
        // Consume hard cap + full burst share.
        assert_eq!(s.record_usage("fg", 1_300).expect("record"), 1_300);
        assert!(!s.admits("fg", 1), "fully bursted: no further admission");
        // Release the full burst; usage returns to exactly the hard cap.
        assert_eq!(s.record_release("fg", 300).expect("release"), 1_000);
        assert_eq!(s.current_usage("fg"), 1_000, "round-trip restores exact prior usage");
        // The freed burst is admissible again.
        assert!(s.admits("fg", 300), "released burst capacity is re-admitted");
        let before = s.current_usage("fg");
        assert!(matches!(s.record_release("fg", u64::MAX), Err(MemoryError::InvalidState(_))));
        assert_eq!(s.current_usage("fg"), before, "over-release must not mutate usage");
    }

    #[test]
    fn huge_pool_and_weights_compute_a_non_wrapping_fair_share() {
        let mut groups = BTreeMap::new();
        groups.insert("heavy".to_string(), group(0, u16::MAX));
        groups.insert("light".to_string(), group(0, 1));
        let s = WeightedResourceGroupScheduler::new(groups, u64::MAX);
        let share = s.fair_burst_share("heavy");
        let expected =
            u64::try_from(u128::from(u64::MAX) * u128::from(u16::MAX) / (u128::from(u16::MAX) + 1))
                .expect("weighted share fits u64");
        assert_eq!(share, expected, "fair share math must stay exact under huge inputs");
        assert!(s.admits("heavy", share), "group may borrow exactly its computed fair share");
        assert!(
            !s.admits("heavy", share.saturating_add(1)),
            "group may not borrow one byte beyond its computed fair share"
        );
    }

    #[test]
    fn admission_reuses_policy_static_fair_share_table_at_high_group_cardinality() {
        let mut groups = BTreeMap::new();
        for index in 0..10_000 {
            groups.insert(format!("group-{index:05}"), group(1_000, 1));
        }
        let mut scheduler = WeightedResourceGroupScheduler::new(groups, 10_000);
        assert_eq!(scheduler.fair_burst_shares.len(), 10_000);
        assert_eq!(scheduler.fair_burst_shares.values().copied().sum::<u64>(), 10_000);

        let allocation = scheduler.fair_burst_shares.clone();
        scheduler.record_usage("group-05000", 1_000).expect("record hard-cap usage");
        for _ in 0..1_000 {
            assert!(scheduler.admits("group-05000", 1));
            assert!(!scheduler.admits("group-05000", 2));
        }
        assert_eq!(
            scheduler.fair_burst_shares, allocation,
            "live admission must not rebuild policy-static shares"
        );
    }

    #[test]
    fn repeated_exact_usage_release_round_trip_never_wraps() {
        let mut s = two_group_scheduler();
        for _ in 0..8 {
            assert_eq!(s.record_usage("fg", u64::MAX).expect("first charge"), u64::MAX);
            assert_eq!(s.current_usage("fg"), u64::MAX);
            assert!(matches!(s.record_usage("fg", 1), Err(MemoryError::Capacity(_))));
            assert_eq!(s.current_usage("fg"), u64::MAX, "overflow must not alter usage");
            assert!(!s.admits("fg", 1), "admission must deny unrepresentable sum");
            assert_eq!(s.record_release("fg", u64::MAX).expect("release"), 0);
            assert_eq!(s.current_usage("fg"), 0);
        }
    }
}
