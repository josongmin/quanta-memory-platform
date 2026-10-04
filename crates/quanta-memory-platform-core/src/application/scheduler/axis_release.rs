use super::*;

fn capped_scheduler() -> WeightedResourceGroupScheduler {
    let mut groups = BTreeMap::new();
    groups.insert(
        "g".to_string(),
        ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 100,
            hard_cap_bytes: 100,
            burst_percent: 0,
            reclaim_priority: crate::domain::policy::ReclaimPriority::Normal,
            admission_weight: 1,
        },
    );
    WeightedResourceGroupScheduler::new(groups, 0)
}

#[test]
fn headroom_release_rejects_wrong_axis_without_reopening_admission() {
    let mut scheduler = capped_scheduler();
    scheduler.record_usage("g", &MemoryClass::ArtifactCache, 100).expect("charge group");
    assert!(!scheduler.admits("g", 1));

    assert!(matches!(
        scheduler.record_release("g", &MemoryClass::Headroom, 50),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(scheduler.current_usage("g"), 100);
    assert_eq!(scheduler.current_non_headroom_usage("g"), 100);
    assert_eq!(scheduler.unused_reserved_floor_bytes(), 0);
    assert!(!scheduler.admits("g", 1), "denied release cannot create group capacity");
}

#[test]
fn headroom_release_accepts_only_the_recorded_headroom_axis() {
    let mut scheduler = capped_scheduler();
    scheduler.record_usage("g", &MemoryClass::ArtifactCache, 50).expect("non-headroom charge");
    scheduler.record_usage("g", &MemoryClass::Headroom, 20).expect("headroom charge");

    assert!(matches!(
        scheduler.record_release("g", &MemoryClass::Headroom, 21),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(scheduler.current_usage("g"), 70);
    assert_eq!(scheduler.current_non_headroom_usage("g"), 50);

    assert_eq!(
        scheduler.record_release("g", &MemoryClass::Headroom, 20).expect("exact headroom release"),
        50
    );
    assert_eq!(scheduler.current_non_headroom_usage("g"), 50);
    assert_eq!(
        scheduler
            .record_release("g", &MemoryClass::ArtifactCache, 50)
            .expect("remaining class release"),
        0
    );
}

#[test]
fn rejected_headroom_release_preserves_unused_group_floor() {
    let mut groups = BTreeMap::new();
    groups.insert(
        "g".to_string(),
        ResourceGroup {
            reserved_bytes: 80,
            soft_cap_bytes: 100,
            hard_cap_bytes: 100,
            burst_percent: 0,
            reclaim_priority: crate::domain::policy::ReclaimPriority::Normal,
            admission_weight: 1,
        },
    );
    let mut scheduler = WeightedResourceGroupScheduler::new(groups, 0);
    scheduler.record_usage("g", &MemoryClass::ArtifactCache, 50).expect("non-headroom usage");
    scheduler.record_usage("g", &MemoryClass::Headroom, 20).expect("headroom usage");
    assert_eq!(scheduler.unused_reserved_floor_bytes(), 30);
    assert!(!scheduler.admits("g", 31));

    assert!(matches!(
        scheduler.record_release("g", &MemoryClass::Headroom, 30),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(scheduler.current_usage("g"), 70);
    assert_eq!(scheduler.current_non_headroom_usage("g"), 50);
    assert_eq!(scheduler.unused_reserved_floor_bytes(), 30);
    assert!(!scheduler.admits("g", 31));

    scheduler.record_release("g", &MemoryClass::Headroom, 20).expect("exact headroom release");
    assert_eq!(scheduler.current_usage("g"), 50);
    assert_eq!(scheduler.unused_reserved_floor_bytes(), 30);
}
