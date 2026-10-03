impl Scheduler {
    fn admits(&self, group: &str, bytes: u64) -> bool {
        // ruleid: qmp-admission-no-share-recompute
        let shares = admission::precompute_fair_burst_shares(&self.groups, self.pool);
        shares.contains_key(group)
    }
    fn new(groups: Groups, pool: u64) -> Self {
        let shares = admission::precompute_fair_burst_shares(&groups, pool);
        Self { groups, shares }
    }
}
