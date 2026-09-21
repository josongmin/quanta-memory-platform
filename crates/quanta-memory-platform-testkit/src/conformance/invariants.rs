//! Host-independent invariant assertions and the contract families they belong
//! to.
//!
//! A conformance failure must localize to a *contract family* (an accounting or
//! lifecycle contract), never to a host package. [`ContractFamily`] is that
//! taxonomy; [`InvariantAssertion`] is a single named, evidence-carrying check
//! whose verdict is attributed to exactly one family.

use std::fmt::{Display, Formatter};

/// The contract family a violated invariant belongs to.
///
/// These are library-level contracts (playbook sections 1 and 5), not host
/// packages. A conformance failure names one of these so an integrator knows
/// which portable contract broke, independent of which host wired it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContractFamily {
    /// `sum(class usage) == sum(owner usage)` (playbook 1.4 invariant 1).
    ClassOwnerBalance,
    /// `sum(owner usage) == payload + metadata + live scratch` (invariant 2).
    OwnerPartition,
    /// Reserve fails closed: a denied reserve does not mutate accounting.
    ReserveFailClosed,
    /// Reservation tickets have exactly one commit and one terminal release.
    TicketLifecycle,
    /// Live writes are rejected while a rebuild window is open (invariant 4).
    RebuildWriteFence,
    /// Pin counts are metadata only; a pinned payload is never deleted (5.1/4.3).
    PinDiscipline,
    /// Reclaim accounting: a reclaim sweep frees real bytes and keeps balance.
    ReclaimAccounting,
    /// Spill controller routes or fails closed without touching broker bytes.
    SpillRouting,
    /// Allocator telemetry is advisory only and never mutates accounting (inv 6).
    AllocatorAdvisory,
}

impl ContractFamily {
    /// Stable label for reporting.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ClassOwnerBalance => "class_owner_balance",
            Self::OwnerPartition => "owner_partition",
            Self::ReserveFailClosed => "reserve_fail_closed",
            Self::TicketLifecycle => "ticket_lifecycle",
            Self::RebuildWriteFence => "rebuild_write_fence",
            Self::PinDiscipline => "pin_discipline",
            Self::ReclaimAccounting => "reclaim_accounting",
            Self::SpillRouting => "spill_routing",
            Self::AllocatorAdvisory => "allocator_advisory",
        }
    }
}

impl Display for ContractFamily {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The outcome of one invariant assertion: which family it tested, whether it
/// held, and an evidence message describing the observed values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssertionVerdict {
    pub family: ContractFamily,
    pub name: String,
    pub held: bool,
    pub evidence: String,
}

impl AssertionVerdict {
    #[must_use]
    pub fn held(&self) -> bool {
        self.held
    }
}

impl Display for AssertionVerdict {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let status = if self.held { "HELD" } else { "VIOLATED" };
        write!(f, "[{}] {} ({}): {}", status, self.name, self.family, self.evidence)
    }
}

/// A single host-independent invariant check.
///
/// An `InvariantAssertion` carries a name, the contract family it defends, and a
/// closure that, given evidence values, decides whether the invariant holds. It
/// never panics: a false predicate yields a `VIOLATED` verdict with the observed
/// evidence so the failure localizes to a contract family rather than aborting.
pub struct InvariantAssertion {
    family: ContractFamily,
    name: String,
}

impl InvariantAssertion {
    #[must_use]
    pub fn new(family: ContractFamily, name: impl Into<String>) -> Self {
        Self { family, name: name.into() }
    }

    #[must_use]
    pub fn family(&self) -> ContractFamily {
        self.family
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Assert that two byte totals are equal. Evidence carries both sides so a
    /// violation says exactly how far apart they are.
    #[must_use]
    pub fn assert_eq_bytes(&self, label: &str, left: u64, right: u64) -> AssertionVerdict {
        let held = left == right;
        let drift = left as i128 - right as i128;
        AssertionVerdict {
            family: self.family,
            name: self.name.clone(),
            held,
            evidence: format!("{label}: left={left} right={right} drift={drift}"),
        }
    }

    /// Assert a boolean predicate, attaching a supplied evidence string.
    #[must_use]
    pub fn assert_that(&self, predicate: bool, evidence: impl Into<String>) -> AssertionVerdict {
        AssertionVerdict {
            family: self.family,
            name: self.name.clone(),
            held: predicate,
            evidence: evidence.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_bytes_hold_unequal_violate_with_drift_evidence() {
        let a = InvariantAssertion::new(ContractFamily::ClassOwnerBalance, "balance");
        let ok = a.assert_eq_bytes("class_vs_owner", 1_000, 1_000);
        assert!(ok.held());
        assert_eq!(ok.family, ContractFamily::ClassOwnerBalance);

        // Negative pair: a real divergence must be VIOLATED and report the drift.
        let bad = a.assert_eq_bytes("class_vs_owner", 1_000, 900);
        assert!(!bad.held());
        assert!(bad.evidence.contains("drift=100"), "evidence must expose the drift: {bad}");
    }

    #[test]
    fn predicate_assertion_carries_supplied_evidence() {
        let a = InvariantAssertion::new(ContractFamily::PinDiscipline, "pinned_not_deleted");
        let v = a.assert_that(false, "delete returned Ok while pin_count=1");
        assert!(!v.held());
        assert_eq!(v.family, ContractFamily::PinDiscipline);
        assert!(v.to_string().contains("VIOLATED"));
    }

    /// The full contract-family taxonomy. The conformance suite localizes every
    /// failure to one of these; if a variant is added, this list (and the suite's
    /// coverage of it) must be revisited.
    const ALL_FAMILIES: &[ContractFamily] = &[
        ContractFamily::ClassOwnerBalance,
        ContractFamily::OwnerPartition,
        ContractFamily::ReserveFailClosed,
        ContractFamily::TicketLifecycle,
        ContractFamily::RebuildWriteFence,
        ContractFamily::PinDiscipline,
        ContractFamily::ReclaimAccounting,
        ContractFamily::SpillRouting,
        ContractFamily::AllocatorAdvisory,
    ];

    #[test]
    fn every_family_has_a_unique_stable_label() {
        // scenario G localization: a failure must name exactly one family, so the
        // labels reporting depends on must be distinct and non-empty. Two families
        // sharing a label would make a violation ambiguous to an integrator.
        let mut labels: Vec<&'static str> = ALL_FAMILIES.iter().map(|f| f.label()).collect();
        let count = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), count, "contract family labels must be unique: {labels:?}");
        assert!(labels.iter().all(|l| !l.is_empty()), "no family label may be empty");
        // Display must agree with label() so reporting is consistent.
        for f in ALL_FAMILIES {
            assert_eq!(f.to_string(), f.label(), "Display must match label for {f:?}");
        }
    }

    #[test]
    fn drift_evidence_sign_distinguishes_over_from_under_reporting() {
        // scenario G evidence: the drift sign must reveal the DIRECTION of the
        // imbalance (left over-reports vs under-reports), not just its magnitude,
        // so an integrator can tell which side lied.
        let a = InvariantAssertion::new(ContractFamily::OwnerPartition, "partition");
        let over = a.assert_eq_bytes("owner_vs_partition", 1_500, 1_000);
        assert!(!over.held());
        assert!(over.evidence.contains("drift=500"), "positive drift: {over}");

        let under = a.assert_eq_bytes("owner_vs_partition", 1_000, 1_500);
        assert!(!under.held());
        assert!(under.evidence.contains("drift=-500"), "negative drift: {under}");

        // Exact equality at the boundary holds with zero drift.
        let exact = a.assert_eq_bytes("owner_vs_partition", u64::MAX, u64::MAX);
        assert!(exact.held(), "equal maxima must hold");
        assert!(exact.evidence.contains("drift=0"), "equal sides report zero drift: {exact}");
    }

    #[test]
    fn family_attribution_is_preserved_through_both_assertion_constructors() {
        // scenario G localization: whichever family an assertion is built for must
        // be the family carried on its verdict, for both the byte and predicate
        // constructors. A misattributed verdict would localize a failure to the
        // wrong portable contract.
        for &family in ALL_FAMILIES {
            let a = InvariantAssertion::new(family, "named");
            assert_eq!(a.family(), family);
            assert_eq!(a.name(), "named");
            let bytes_v = a.assert_eq_bytes("x", 1, 2);
            assert_eq!(bytes_v.family, family, "assert_eq_bytes attribution");
            let pred_v = a.assert_that(true, "ok");
            assert_eq!(pred_v.family, family, "assert_that attribution");
        }
    }
}
