use super::*;

#[cfg(feature = "fixtures")]
pub(super) fn make_platform(kit: &MemoryTestKit) -> Result<LiveMemoryPlatform, String> {
    MemoryPlatformBuilder::new()
        .with_manifest(kit.fixture.manifest.clone())
        .build()
        .map_err(|error| error.to_string())
}

#[cfg(feature = "fixtures")]
pub(super) fn binding(class: MemoryClass, owner: &str) -> StoreBindingHandle {
    StoreBindingHandle::new(class, owner.to_string(), None)
}

#[cfg(feature = "fixtures")]
pub(super) fn broker_bytes(broker: &LiveProcessMemoryBroker) -> Result<u64, String> {
    broker.ledger_snapshot().map(|snapshot| snapshot.total_owner_bytes()).map_err(|e| e.to_string())
}

#[cfg(feature = "fixtures")]
pub(super) fn session_transcript_budget(
    base: &MemoryTestKit,
    hard_cap_bytes: u64,
) -> MemoryTestKit {
    let mut manifest = base.fixture.manifest.clone();
    let fallback = MemoryClassBudget {
        reserved_bytes: 0,
        soft_cap_bytes: hard_cap_bytes,
        hard_cap_bytes,
        borrow_from_shared_pool: false,
        reclaim_priority: ReclaimPriority::Last,
    };
    let prior = manifest
        .residency
        .classes
        .get(&MemoryClass::SessionTranscript)
        .cloned()
        .unwrap_or(fallback);
    manifest.residency = manifest.residency.clone().with_class_budget(
        MemoryClass::SessionTranscript,
        MemoryClassBudget { soft_cap_bytes: hard_cap_bytes, hard_cap_bytes, ..prior },
    );
    MemoryTestKit { fixture: MemoryFixture { manifest, fake_pressure: base.fixture.fake_pressure } }
}

/// A kit whose `class` is tightly capped at `hard_cap_bytes`, for driving
/// over-cap / atomic-recharge cases on a specific store class.
#[cfg(feature = "fixtures")]
pub(super) fn class_capped_kit(
    base: &MemoryTestKit,
    class: MemoryClass,
    hard_cap_bytes: u64,
) -> MemoryTestKit {
    let mut manifest = base.fixture.manifest.clone();
    manifest.residency = manifest.residency.clone().with_class_budget(
        class,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: hard_cap_bytes,
            hard_cap_bytes,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Last,
        },
    );
    MemoryTestKit { fixture: MemoryFixture { manifest, fake_pressure: base.fixture.fake_pressure } }
}

#[cfg(feature = "fixtures")]
pub(super) fn run_payload_store_slice(kit: &MemoryTestKit) -> Vec<AssertionVerdict> {
    // The returned reference is part of the retained charge. Use its observed
    // wire length, then independently add the body and semantic index/state
    // components rather than calling the store's cost calculator.
    let accounting =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "payload_dedup_refund");
    let pin_discipline = InvariantAssertion::new(ContractFamily::PinDiscipline, "pin_safe_delete");
    let platform = match make_platform(kit) {
        Ok(platform) => platform,
        Err(err) => {
            return vec![
                accounting.assert_that(false, format!("payload broker setup failed: {err}")),
                pin_discipline
                    .assert_that(false, "payload pin-safe delete skipped: broker setup failed"),
            ];
        }
    };
    let broker = platform.broker().clone();
    let store = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(binding(MemoryClass::ArtifactCache, "conformance:payload"))
        .build()
        .expect("bound payload builder");
    let payload = vec![7u8; 32];
    let first = match store
        .put_canonical(quanta_memory_platform_core::RetainedEnvelope::new(payload.clone()))
    {
        Ok(published) => published,
        Err(err) => {
            return vec![
                accounting.assert_that(false, format!("first payload put failed: {err:?}")),
                pin_discipline
                    .assert_that(false, "payload pin-safe delete skipped: initial put failed"),
            ];
        }
    };
    let expected_entry_bytes =
        32 + u64::try_from(first.payload_ref().len()).expect("reference length fits u64") + 64 + 16;
    let ledger_after_first = broker_bytes(&broker);
    let second = store.put_canonical(quanta_memory_platform_core::RetainedEnvelope::new(payload));
    let ledger_after_dedup = broker_bytes(&broker);
    let charged_after_dedup = store.charged_bytes().map_err(|e| e.to_string());
    let pin = store.pin(first.payload_ref());
    let ledger_before_pinned_delete = broker_bytes(&broker);
    let pinned_delete = store.delete_if_unpinned(first.payload_ref());
    let ledger_after_pinned_delete = broker_bytes(&broker);
    let charged_after_pinned_delete = store.charged_bytes().map_err(|e| e.to_string());
    let unpin = store.unpin(first.payload_ref());
    let final_delete = store.delete_if_unpinned(first.payload_ref());
    let final_ledger = broker_bytes(&broker);
    let final_charged = store.charged_bytes().map_err(|e| e.to_string());

    vec![
        accounting.assert_that(
            matches!(&second, Ok(published) if published.payload_ref() == first.payload_ref())
                && matches!(&ledger_after_first, Ok(bytes) if *bytes == expected_entry_bytes)
                && matches!(&ledger_after_dedup, Ok(bytes) if *bytes == expected_entry_bytes)
                && matches!(&charged_after_dedup, Ok(bytes) if *bytes == expected_entry_bytes)
                && final_delete.is_ok()
                && matches!(&final_ledger, Ok(bytes) if *bytes == 0)
                && matches!(&final_charged, Ok(bytes) if *bytes == 0),
            format!(
                "first_ref={} second={second:?} ledger_after_first={ledger_after_first:?} ledger_after_dedup={ledger_after_dedup:?} charged_after_dedup={charged_after_dedup:?} final_delete={final_delete:?} final_ledger={final_ledger:?} final_charged={final_charged:?}",
                first.payload_ref()
            ),
        ),
        pin_discipline.assert_that(
            pin.is_ok()
                && pinned_delete.is_err()
                && matches!(
                    (
                        &ledger_before_pinned_delete,
                        &ledger_after_pinned_delete,
                        &charged_after_pinned_delete
                    ),
                    (Ok(before), Ok(after), Ok(charged)) if *before == *after && *charged == *before
                )
                && unpin.is_ok()
                // The full discipline cycle belongs to THIS family: after unpin,
                // the delete must SUCCEED and refund (so a store that refuses a
                // delete even after unpin is localized to PinDiscipline, not only
                // to the accounting family).
                && final_delete.is_ok()
                && matches!(&final_ledger, Ok(bytes) if *bytes == 0)
                && matches!(&final_charged, Ok(bytes) if *bytes == 0),
            format!(
                "pin={pin:?} pinned_delete={pinned_delete:?} ledger_before_pinned_delete={ledger_before_pinned_delete:?} ledger_after_pinned_delete={ledger_after_pinned_delete:?} charged_after_pinned_delete={charged_after_pinned_delete:?} unpin={unpin:?} final_delete={final_delete:?} final_ledger={final_ledger:?} final_charged={final_charged:?}",
            ),
        ),
    ]
}

#[cfg(feature = "fixtures")]
pub(super) fn run_retained_map_slice(kit: &MemoryTestKit) -> Vec<AssertionVerdict> {
    let accounting =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "retained_map_refunds");
    let platform = match make_platform(kit) {
        Ok(platform) => platform,
        Err(err) => {
            return vec![
                accounting.assert_that(false, format!("retained-map broker setup failed: {err}"))
            ]
        }
    };
    run_retained_map_slice_with_platform(kit, &platform)
}

#[cfg(feature = "fixtures")]
pub(super) fn run_retained_map_slice_with_platform(
    kit: &MemoryTestKit,
    platform: &LiveMemoryPlatform,
) -> Vec<AssertionVerdict> {
    let accounting =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "retained_map_refunds");
    let broker = platform.broker();
    let store = platform
        .retained_map_store_builder::<String, Vec<u8>>()
        .bind(binding(MemoryClass::MemoResidency, "conformance:map"))
        .build();

    let overwrite = store
        .insert_checked("k".to_string(), vec![0u8; 24])
        .and_then(|_| store.insert_checked("k".to_string(), vec![1u8; 10]));
    let after_overwrite = broker_bytes(broker);
    let committed_after_overwrite = store.committed_bytes().map_err(|e| e.to_string());
    let seed_sweep = store
        .insert_checked("b".to_string(), vec![2u8; 7])
        .and_then(|_| store.insert_checked("c".to_string(), vec![3u8; 5]));
    let sweep_stats = store.sweep(SweepHint { target_bytes: 11 });
    let after_sweep = broker_bytes(broker);
    let committed_after_sweep = store.committed_bytes().map_err(|e| e.to_string());

    // Tight-cap overwrite: an in-place replace whose NET fits the class cap but
    // whose (old + new) transiently exceeds it. An atomic recharge admits it; a
    // non-atomic charge-then-refund regression would be denied. This is the only
    // way to localize a non-atomic recharge bug to the accounting family.
    let recharge = InvariantAssertion::new(
        ContractFamily::ReclaimAccounting,
        "retained_atomic_recharge_overwrite",
    );
    let recharge_verdict =
        match make_platform(&class_capped_kit(kit, MemoryClass::MemoResidency, 70)) {
            Ok(tight_platform) => {
                let tight_broker = tight_platform.broker();
                let tight_store = tight_platform
                    .retained_map_store_builder::<String, Vec<u8>>()
                    .bind(binding(MemoryClass::MemoResidency, "conformance:map-tight"))
                    .build();
                // key 1 + hash slot 24 + state 16: old=65, new=61.
                // Combined transient 126 exceeds cap 70; net new 61 fits.
                let seed = tight_store.insert_checked("k".to_string(), vec![0u8; 24]);
                let net_fit = tight_store.insert_checked("k".to_string(), vec![0u8; 20]);
                let tight_ledger = broker_bytes(tight_broker);
                recharge.assert_that(
                    seed.is_ok()
                        && net_fit.is_ok()
                        && matches!(&tight_ledger, Ok(bytes) if *bytes == 61),
                    format!("seed={seed:?} net_fit={net_fit:?} tight_ledger={tight_ledger:?}"),
                )
            }
            Err(err) => {
                recharge.assert_that(false, format!("tight retained broker setup failed: {err}"))
            }
        };

    vec![
        accounting.assert_that(
            overwrite.is_ok()
                && matches!(&after_overwrite, Ok(bytes) if *bytes == 51)
                && matches!(&committed_after_overwrite, Ok(bytes) if *bytes == 51)
                && seed_sweep.is_ok()
                && matches!(&sweep_stats, Ok(stats) if stats.reclaimed_items == 1 && stats.reclaimed_bytes == 46)
                && matches!(&after_sweep, Ok(bytes) if *bytes == 99)
                && matches!(&committed_after_sweep, Ok(bytes) if *bytes == 99)
                && matches!(store.get_checked(&"b".to_string()), Ok(Some(value)) if value == vec![2u8; 7])
                && matches!(store.get_checked(&"c".to_string()), Ok(None))
                && matches!(store.get_checked(&"k".to_string()), Ok(Some(value)) if value == vec![1u8; 10]),
            format!(
                "overwrite={overwrite:?} after_overwrite={after_overwrite:?} committed_after_overwrite={committed_after_overwrite:?} seed_sweep={seed_sweep:?} sweep_stats={sweep_stats:?} after_sweep={after_sweep:?} committed_after_sweep={committed_after_sweep:?} remaining_keys={:?}",
                store.keys_checked()
            ),
        ),
        recharge_verdict,
    ]
}

#[cfg(feature = "fixtures")]
pub(super) fn run_transcript_slice(kit: &MemoryTestKit) -> Vec<AssertionVerdict> {
    let session = 1 + 24 + 16;
    let evidence = 2 * 2 + 2 * 24 + 2 * 32 + 2 * 16;
    let first_active = 2 + 2 + 24 + 16;
    let second_active = 8 + 2 + 24 + 16;
    let before_expected = session + evidence + first_active + evidence + second_active;
    let after_expected = before_expected - second_active;
    let accounting =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "transcript_truncate_refund");
    let fail_closed = InvariantAssertion::new(
        ContractFamily::ReserveFailClosed,
        "transcript_over_cap_fail_closed",
    );

    let platform = match make_platform(kit) {
        Ok(platform) => platform,
        Err(err) => {
            return vec![
                accounting.assert_that(false, format!("transcript broker setup failed: {err}")),
                fail_closed
                    .assert_that(false, "transcript over-cap check skipped: broker setup failed"),
            ];
        }
    };
    let broker = platform.broker().clone();
    let store = platform
        .transcript_store_builder::<String, String, String>()
        .bind(binding(MemoryClass::SessionTranscript, "conformance:transcript"))
        .build()
        .expect("bound transcript builder");
    let truncate = store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new("s".to_string(), 0, "m0".to_string()),
            "aa".to_string(),
        ))
        .and_then(|_| {
            store.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new("s".to_string(), 1, "m1".to_string()),
                "bbbbbbbb".to_string(),
            ))
        });
    let before_truncate = broker_bytes(&broker);
    let truncate_result = store.truncate_to(&"s".to_string(), 1);
    let after_truncate = broker_bytes(&broker);
    let retained_after_truncate = store.retained_bytes().map_err(|e| e.to_string());

    let tiny_kit = session_transcript_budget(kit, 4);
    let tiny_platform = match make_platform(&tiny_kit) {
        Ok(platform) => platform,
        Err(err) => {
            return vec![
                accounting.assert_that(
                    truncate.is_ok()
                        && matches!(&before_truncate, Ok(bytes) if *bytes == before_expected)
                        && truncate_result.is_ok()
                        && matches!(&after_truncate, Ok(bytes) if *bytes == after_expected)
                        && matches!(&retained_after_truncate, Ok(bytes) if *bytes == after_expected),
                    format!(
                        "truncate={truncate:?} before_truncate={before_truncate:?} truncate_result={truncate_result:?} after_truncate={after_truncate:?} retained_after_truncate={retained_after_truncate:?}",
                    ),
                ),
                fail_closed.assert_that(false, format!("tiny transcript broker setup failed: {err}")),
            ];
        }
    };
    let tiny_broker = tiny_platform.broker().clone();
    let tiny_store = tiny_platform
        .transcript_store_builder::<String, String, String>()
        .bind(binding(MemoryClass::SessionTranscript, "conformance:transcript-tight"))
        .build()
        .expect("bound transcript builder");
    let over_cap = tiny_store.append(TranscriptAppendRequestV1::new(
        TranscriptAppendIdentityV1::new("s".to_string(), 0, "m0".to_string()),
        "toolong".to_string(),
    ));
    let tiny_ledger_after = broker_bytes(&tiny_broker);
    let tiny_retained_after = tiny_store.retained_bytes().map_err(|e| e.to_string());

    vec![
        accounting.assert_that(
            truncate.is_ok()
                && matches!(&before_truncate, Ok(bytes) if *bytes == before_expected)
                && truncate_result.is_ok()
                && matches!(&after_truncate, Ok(bytes) if *bytes == after_expected)
                && matches!(&retained_after_truncate, Ok(bytes) if *bytes == after_expected),
            format!(
                "truncate={truncate:?} before_truncate={before_truncate:?} truncate_result={truncate_result:?} after_truncate={after_truncate:?} retained_after_truncate={retained_after_truncate:?}",
            ),
        ),
        fail_closed.assert_that(
            matches!(
                &over_cap,
                Err(TranscriptAppendErrorV1::Accounting(MemoryError::Capacity(_)))
            )
                && matches!(&tiny_ledger_after, Ok(bytes) if *bytes == 0)
                && matches!(&tiny_retained_after, Ok(bytes) if *bytes == 0)
                && matches!(tiny_store.stats(), Ok(stats) if stats.item_count == 0),
            format!(
                "over_cap={over_cap:?} tiny_ledger_after={tiny_ledger_after:?} tiny_retained_after={tiny_retained_after:?} tiny_stats={:?}",
                tiny_store.stats()
            ),
        ),
    ]
}

/// The SpillRouting round-trip verdict over an INJECTED controller, so the suite
/// can be driven against a broken (handle-dropping) target and proven to BITE —
/// not merely accept the honest in-memory target. A spill then restore must hand
/// back the same handle (canonical target inventory 1 -> 0).
#[cfg(feature = "fixtures")]
pub(super) fn spill_round_trip_verdict<T: SpillTarget<String>>(
    online: &RoutedSpillController<T>,
) -> AssertionVerdict {
    let spill = InvariantAssertion::new(ContractFamily::SpillRouting, "spill_round_trip");
    let spill_ok = online.spill("handle-a".to_string());
    // Capture inventory BETWEEN spill and restore: a target that drops the handle
    // would survive an "after == 0" check alone. The round-trip must go 1 -> 0.
    let before_restore = online.spilled_count();
    let restore = online.restore();
    let after = online.spilled_count();
    spill.assert_that(
        spill_ok.is_ok()
            && before_restore == Ok(1)
            && matches!(&restore, Ok(Some(handle)) if handle == "handle-a")
            && after == Ok(0),
        format!(
            "spill_ok={spill_ok:?} before_restore={before_restore:?} restore={restore:?} after={after:?}"
        ),
    )
}

#[cfg(feature = "fixtures")]
pub(super) fn spill_offline_restore_verdict<T: SpillTarget<String>>(
    offline: &RoutedSpillController<T>,
) -> AssertionVerdict {
    let assertion =
        InvariantAssertion::new(ContractFamily::SpillRouting, "spill_offline_restore_fail_closed");
    let before = offline.spilled_count();
    let restored = offline.restore();
    let after = offline.spilled_count();
    assertion.assert_that(
        before == Ok(1) && matches!(&restored, Err(MemoryError::Capacity(_))) && after == before,
        format!("before={before:?} restore={restored:?} after={after:?}"),
    )
}

#[cfg(feature = "fixtures")]
pub(super) fn run_spill_slice() -> Vec<AssertionVerdict> {
    let fail_closed =
        InvariantAssertion::new(ContractFamily::SpillRouting, "spill_unavailable_fail_closed");

    // The round-trip check goes through the SAME injectable helper a broken-target
    // self-test drives, so weakening it would flip that self-test (it BITES).
    let online = RoutedSpillController::new(InMemorySpillTarget::available());
    let round_trip = spill_round_trip_verdict(&online);

    let offline = RoutedSpillController::new(InMemorySpillTarget::unavailable());
    let spill_denied = offline.spill("handle-b".to_string());
    let offline_spilled_after = offline.spilled_count();

    let retained = InMemorySpillTarget::available();
    let seeded = retained.accept("retained-through-offline".to_string());
    retained.set_available(false);
    let offline_restore = if seeded.is_ok() {
        spill_offline_restore_verdict(&RoutedSpillController::new(retained))
    } else {
        InvariantAssertion::new(ContractFamily::SpillRouting, "spill_offline_restore_fail_closed")
            .assert_that(false, format!("failed to seed offline restore: {seeded:?}"))
    };

    vec![
        round_trip,
        offline_restore,
        fail_closed.assert_that(
            matches!(&spill_denied, Err(failure) if matches!(failure.error(), MemoryError::Capacity(_)) && failure.handle() == "handle-b")
                && offline_spilled_after == Ok(0),
            format!("spill_denied={spill_denied:?} offline_spilled_after={offline_spilled_after:?}",),
        ),
    ]
}
