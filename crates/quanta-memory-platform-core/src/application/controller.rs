//! Deterministic pressure controller and reclaim ladder.
//!
//! The frozen [`PressureController`] trait exposes the current pressure state
//! and an allocator-snapshot observation hook. [`LadderPressureController`] is
//! the concrete deterministic implementation: it classifies pressure purely
//! from *semantic* usage ratios (allocator telemetry is advisory and never
//! changes the classification) and exposes a fixed reclaim ladder so the broker
//! slow path knows where to start reclaiming.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::domain::policy::ReclaimPriority;
use crate::{AllocatorSnapshot, MemoryError, MemoryResult, PressureState};

/// Pressure classification + advisory allocator observation.
pub trait PressureController {
    fn current_pressure(&self) -> PressureState;
    fn observe_allocator_snapshot(&self, snapshot: &AllocatorSnapshot);
}

/// A rung of the reclaim ladder. The slow path walks rungs in order until the
/// requested delta fits or the ladder is exhausted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReclaimRung {
    /// Reclaim from the same class's most-recently-grown owner first.
    SameClassRecentGrow,
    /// Reclaim from the same class's general reclaimables.
    SameClassGeneral,
    /// Reclaim from lower-priority classes.
    LowerPriorityClasses,
    /// Spill spillable classes to backing store.
    Spill,
}

/// Deterministic ladder thresholds expressed as percent of the global hard
/// limit. `soft <= hard <= fatal` is required and validated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PressureThresholds {
    pub soft_percent: u8,
    pub hard_percent: u8,
    pub fatal_percent: u8,
}

impl Default for PressureThresholds {
    fn default() -> Self {
        Self { soft_percent: 75, hard_percent: 90, fatal_percent: 98 }
    }
}

impl PressureThresholds {
    /// Construct a validated set of ladder thresholds.
    ///
    /// The ladder is only coherent when the rungs are monotonic
    /// (`soft <= hard <= fatal`) and every percent is a real fraction of the
    /// limit (`<= 100`). Inverted or out-of-range thresholds are rejected
    /// fail-closed with a [`MemoryError::Config`] rather than silently producing
    /// an unreachable rung (e.g. a `fatal_percent` below `hard_percent`, which
    /// would make `Hard` unreachable).
    pub fn new(soft_percent: u8, hard_percent: u8, fatal_percent: u8) -> MemoryResult<Self> {
        let candidate = Self { soft_percent, hard_percent, fatal_percent };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Validate monotonicity and bounds. Returns the typed error describing the
    /// first violated constraint so callers can surface a precise reason.
    pub fn validate(&self) -> MemoryResult<()> {
        if self.fatal_percent > 100 || self.hard_percent > 100 || self.soft_percent > 100 {
            return Err(MemoryError::config(format!(
                "pressure thresholds must be percentages <= 100 (soft={}, hard={}, fatal={})",
                self.soft_percent, self.hard_percent, self.fatal_percent
            )));
        }
        if !self.is_monotonic() {
            return Err(MemoryError::config(format!(
                "pressure thresholds must be monotonic soft<=hard<=fatal (soft={}, hard={}, fatal={})",
                self.soft_percent, self.hard_percent, self.fatal_percent
            )));
        }
        Ok(())
    }

    #[must_use]
    pub fn is_monotonic(&self) -> bool {
        self.soft_percent <= self.hard_percent && self.hard_percent <= self.fatal_percent
    }
}

/// Concrete pressure controller with a fixed reclaim ladder.
///
/// State is held in an `AtomicU8` so `&self` observation can update the cached
/// pressure classification without interior `&mut` (the broker shares it behind
/// an `Arc`).
#[derive(Debug)]
pub struct LadderPressureController {
    global_hard_limit_bytes: u64,
    thresholds: PressureThresholds,
    pressure: AtomicU8,
}

impl LadderPressureController {
    #[must_use]
    pub fn new(
        global_hard_limit_bytes: u64,
        thresholds: PressureThresholds,
        initial: PressureState,
    ) -> Self {
        Self {
            global_hard_limit_bytes,
            thresholds,
            pressure: AtomicU8::new(encode_pressure(initial)),
        }
    }

    /// Classify pressure deterministically from *semantic* used bytes. This is
    /// the SSOT path: allocator telemetry never feeds into it.
    #[must_use]
    pub fn classify_semantic(&self, used_bytes: u64) -> PressureState {
        if self.global_hard_limit_bytes == 0 {
            // Degenerate "unbounded" limit: with a zero budget there is no
            // headroom, so any positive usage is immediately Fatal, but a
            // zero-used system is genuinely under no pressure. Returning
            // Normal for the empty case (rather than a permanent Fatal)
            // keeps classification coherent and idempotent at the origin.
            return if used_bytes == 0 { PressureState::Normal } else { PressureState::Fatal };
        }
        // Compute the ratio in u128 so a large `used_bytes * 100` cannot saturate
        // and UNDER-report the percentage — saturation here would round pressure
        // DOWN (the unsafe direction), masking real memory pressure at very large
        // (exabyte-scale) budgets. u128 cannot overflow for u64 inputs * 100.
        let percent =
            u64::try_from(u128::from(used_bytes) * 100 / u128::from(self.global_hard_limit_bytes))
                .unwrap_or(u64::MAX);
        if percent >= u64::from(self.thresholds.fatal_percent) {
            PressureState::Fatal
        } else if percent >= u64::from(self.thresholds.hard_percent) {
            PressureState::Hard
        } else if percent >= u64::from(self.thresholds.soft_percent) {
            PressureState::Soft
        } else {
            PressureState::Normal
        }
    }

    /// Update the cached classification from semantic used bytes and return it.
    pub fn update_from_semantic(&self, used_bytes: u64) -> PressureState {
        let state = self.classify_semantic(used_bytes);
        self.pressure.store(encode_pressure(state), Ordering::SeqCst);
        state
    }

    /// The reclaim ladder starting point for the current pressure state.
    /// Higher pressure starts the ladder lower (more aggressive).
    #[must_use]
    pub fn ladder_for(&self, state: PressureState) -> Vec<ReclaimRung> {
        match state {
            PressureState::Normal | PressureState::Soft => {
                vec![ReclaimRung::SameClassRecentGrow, ReclaimRung::SameClassGeneral]
            }
            PressureState::Hard => vec![
                ReclaimRung::SameClassRecentGrow,
                ReclaimRung::SameClassGeneral,
                ReclaimRung::LowerPriorityClasses,
            ],
            PressureState::Fatal => vec![
                ReclaimRung::SameClassRecentGrow,
                ReclaimRung::SameClassGeneral,
                ReclaimRung::LowerPriorityClasses,
                ReclaimRung::Spill,
            ],
        }
    }

    /// Deterministic reclaim model. Returns how many bytes can be reclaimed
    /// toward `target_bytes` given the reclaimable pool. Under `Fatal` pressure
    /// with no spillable backing, nothing can be reclaimed: returns zero so the
    /// broker fails closed rather than pretending to free memory.
    #[must_use]
    pub fn reclaimable_bytes(
        &self,
        state: PressureState,
        target_bytes: u64,
        general_reclaimable: u64,
        spillable: u64,
        priority: ReclaimPriority,
    ) -> u64 {
        // Pinned / First-priority classes never yield to reclaim of others.
        let mut pool = general_reclaimable;
        if matches!(state, PressureState::Fatal) {
            // Under fatal pressure, only spillable backing can still be freed;
            // non-spillable reclaimables are already exhausted by definition.
            if spillable == 0 {
                return 0;
            }
            pool = spillable;
        }
        // ReclaimPriority::First classes are reclaimed last conceptually; a
        // First-priority owner contributes nothing to *its own* relief here.
        if matches!(priority, ReclaimPriority::First) && !matches!(state, PressureState::Fatal) {
            return 0;
        }
        pool.min(target_bytes)
    }
}

impl PressureController for LadderPressureController {
    fn current_pressure(&self) -> PressureState {
        decode_pressure(self.pressure.load(Ordering::SeqCst))
    }

    /// Advisory only: observing the allocator snapshot must NOT change the
    /// semantic pressure classification. We intentionally do not mutate the
    /// cached pressure here; this hook exists for telemetry export, and the
    /// SSOT remains [`Self::update_from_semantic`].
    fn observe_allocator_snapshot(&self, _snapshot: &AllocatorSnapshot) {
        // No-op on semantic state by the advisory telemetry contract.
    }
}

fn encode_pressure(state: PressureState) -> u8 {
    match state {
        PressureState::Normal => 0,
        PressureState::Soft => 1,
        PressureState::Hard => 2,
        PressureState::Fatal => 3,
    }
}

fn decode_pressure(raw: u8) -> PressureState {
    match raw {
        0 => PressureState::Normal,
        1 => PressureState::Soft,
        2 => PressureState::Hard,
        _ => PressureState::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> LadderPressureController {
        LadderPressureController::new(
            1_000,
            PressureThresholds::default(), // 75/90/98
            PressureState::Normal,
        )
    }

    #[test]
    fn classification_is_deterministic_across_the_ladder() {
        let c = controller();
        assert_eq!(c.classify_semantic(0), PressureState::Normal);
        assert_eq!(c.classify_semantic(740), PressureState::Normal); // 74%
        assert_eq!(c.classify_semantic(750), PressureState::Soft); // 75%
        assert_eq!(c.classify_semantic(899), PressureState::Soft); // 89%
        assert_eq!(c.classify_semantic(900), PressureState::Hard); // 90%
        assert_eq!(c.classify_semantic(980), PressureState::Fatal); // 98%
                                                                    // re-run is identical (determinism)
        assert_eq!(c.classify_semantic(900), PressureState::Hard);
    }

    #[test]
    fn update_caches_current_pressure() {
        let c = controller();
        assert_eq!(c.current_pressure(), PressureState::Normal);
        c.update_from_semantic(950);
        assert_eq!(c.current_pressure(), PressureState::Hard);
    }

    #[test]
    fn ladder_gets_more_aggressive_with_pressure() {
        let c = controller();
        assert_eq!(
            c.ladder_for(PressureState::Soft),
            vec![ReclaimRung::SameClassRecentGrow, ReclaimRung::SameClassGeneral]
        );
        let fatal = c.ladder_for(PressureState::Fatal);
        assert!(fatal.contains(&ReclaimRung::Spill), "fatal ladder reaches spill");
        assert!(fatal.len() > c.ladder_for(PressureState::Soft).len());
    }

    // --- Negative path 2: reclaim returns zero under hard/fatal pressure with
    // nothing spillable.
    #[test]
    fn reclaim_returns_zero_under_fatal_pressure_without_spill() {
        let c = controller();
        let got = c.reclaimable_bytes(
            PressureState::Fatal,
            500,    // target
            10_000, // general reclaimable (irrelevant under fatal)
            0,      // spillable = 0
            ReclaimPriority::Normal,
        );
        assert_eq!(got, 0, "fatal pressure with no spillable backing reclaims nothing");
    }

    // --- positive pair: under fatal pressure spillable backing CAN be reclaimed
    #[test]
    fn reclaim_uses_spillable_under_fatal_pressure() {
        let c = controller();
        let got = c.reclaimable_bytes(
            PressureState::Fatal,
            500,
            0,
            800, // spillable available
            ReclaimPriority::Normal,
        );
        assert_eq!(got, 500, "fatal pressure can still spill up to the target");
    }

    #[test]
    fn first_priority_owner_does_not_self_reclaim_below_fatal() {
        let c = controller();
        let got = c.reclaimable_bytes(PressureState::Hard, 500, 10_000, 0, ReclaimPriority::First);
        assert_eq!(got, 0, "First-priority class yields nothing under non-fatal pressure");
        // negative pair: a Normal-priority class with the same pool does reclaim.
        let normal =
            c.reclaimable_bytes(PressureState::Hard, 500, 10_000, 0, ReclaimPriority::Normal);
        assert_eq!(normal, 500);
    }

    // scenario E: validated constructor accepts a monotonic ladder and the
    // Default is itself valid.
    #[test]
    fn thresholds_new_accepts_monotonic_and_default_is_valid() {
        let ok = PressureThresholds::new(75, 90, 98).expect("monotonic thresholds are accepted");
        assert_eq!(
            ok,
            PressureThresholds { soft_percent: 75, hard_percent: 90, fatal_percent: 98 }
        );
        // Equal rungs are still monotonic (non-strict) and must be accepted.
        PressureThresholds::new(50, 50, 50).expect("equal rungs are monotonic");
        // The Default must remain usable: it constructs and validates.
        assert!(PressureThresholds::default().is_monotonic());
        PressureThresholds::default().validate().expect("Default thresholds must be valid");
    }

    // scenario E: inverted / out-of-range ladders are rejected fail-closed with
    // the typed Config variant (assert on the variant, not the message).
    #[test]
    fn thresholds_new_rejects_inverted_or_out_of_range() {
        // Table of degenerate inputs that must each be rejected.
        let cases: &[(u8, u8, u8, &str)] = &[
            (90, 75, 98, "soft > hard"),
            (75, 98, 90, "hard > fatal"),
            (98, 90, 75, "fully inverted"),
            (75, 90, 101, "fatal out of range"),
            (0, 200, 250, "hard & fatal out of range"),
        ];
        for &(s, h, f, why) in cases {
            let err = PressureThresholds::new(s, h, f);
            assert!(
                matches!(err, Err(MemoryError::Config(_))),
                "{why}: ({s},{h},{f}) must be rejected as Config, got {err:?}"
            );
        }
    }

    // scenario E: zero global hard limit is the degenerate "unbounded" case.
    // Empty usage is Normal (no pressure), any positive usage is Fatal, and the
    // empty classification is idempotent.
    #[test]
    fn zero_limit_is_normal_when_empty_and_fatal_when_used() {
        let c =
            LadderPressureController::new(0, PressureThresholds::default(), PressureState::Normal);
        assert_eq!(c.classify_semantic(0), PressureState::Normal, "zero-used zero-limit is Normal");
        assert_eq!(
            c.classify_semantic(1),
            PressureState::Fatal,
            "any usage over zero limit is Fatal"
        );
        assert_eq!(c.classify_semantic(u64::MAX), PressureState::Fatal);
        // idempotent at the origin.
        assert_eq!(c.classify_semantic(0), c.classify_semantic(0));
    }

    #[test]
    fn tiny_non_zero_limit_saturates_percent_without_wrapping() {
        let c =
            LadderPressureController::new(1, PressureThresholds::default(), PressureState::Normal);
        assert_eq!(c.classify_semantic(1), PressureState::Fatal, "1/1 is already fatal");
        assert_eq!(
            c.classify_semantic(u64::MAX),
            PressureState::Fatal,
            "used_bytes * 100 must saturate, never wrap below fatal"
        );
    }

    // scenario E: the ladder transitions in BOTH directions. Driving usage up to
    // Hard then back down returns through Soft to Normal, with no hysteresis
    // (classification is a pure function of the current used ratio).
    #[test]
    fn ladder_transitions_up_then_down_through_every_rung() {
        let c = controller(); // limit 1_000, thresholds 75/90/98
                              // Ascending: Normal -> Soft -> Hard -> Fatal.
        let ascending: &[(u64, PressureState)] = &[
            (0, PressureState::Normal),
            (750, PressureState::Soft),
            (900, PressureState::Hard),
            (980, PressureState::Fatal),
        ];
        for &(used, want) in ascending {
            assert_eq!(c.update_from_semantic(used), want, "ascend used={used}");
            assert_eq!(c.current_pressure(), want, "cached state tracks ascend used={used}");
        }
        // Descending back down returns symmetrically: no hysteresis.
        let descending: &[(u64, PressureState)] =
            &[(900, PressureState::Hard), (750, PressureState::Soft), (0, PressureState::Normal)];
        for &(used, want) in descending {
            assert_eq!(c.update_from_semantic(used), want, "descend used={used}");
            assert_eq!(c.current_pressure(), want, "cached state tracks descend used={used}");
        }
    }

    // scenario E: update_from_semantic is idempotent — feeding the same used
    // value twice yields the same state and leaves the cached state identical.
    #[test]
    fn update_from_semantic_is_idempotent() {
        let c = controller();
        let first = c.update_from_semantic(910); // Hard
        let cached_after_first = c.current_pressure();
        let second = c.update_from_semantic(910);
        let cached_after_second = c.current_pressure();
        assert_eq!(first, PressureState::Hard);
        assert_eq!(first, second, "same used yields same returned state");
        assert_eq!(
            cached_after_first, cached_after_second,
            "repeated identical update must not perturb cached state"
        );
    }

    #[test]
    fn allocator_snapshot_observation_does_not_change_pressure() {
        let c = controller();
        c.update_from_semantic(950); // Hard
        assert_eq!(c.current_pressure(), PressureState::Hard);
        // Allocator says everything is fine.
        let snap = AllocatorSnapshot { resident_bytes: Some(0), ..AllocatorSnapshot::default() };
        c.observe_allocator_snapshot(&snap);
        assert_eq!(
            c.current_pressure(),
            PressureState::Hard,
            "advisory allocator observation must not downgrade semantic pressure"
        );
    }
}
