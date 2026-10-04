mod cases_01;
mod tombstone_reservation;

use super::*;
use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
use std::sync::{Arc, Barrier};
use std::thread;

#[derive(Debug)]
struct TestUnbudgeted;

type TestService = PreparedPublicationService<Vec<u8>, TestUnbudgeted>;

fn binding() -> StoreBindingHandle {
    StoreBindingHandle::new(
        MemoryClass::ManifestResidency,
        "prepared-publication-tests".to_string(),
        None,
    )
}

fn service_with_limits(capacity_bytes: Option<u64>, limits: Option<(usize, u64)>) -> TestService {
    PreparedPublicationService::with_accounting(
        binding(),
        capacity_bytes,
        limits,
        StoreAccounting::Unbudgeted,
    )
}

fn service(capacity_bytes: Option<u64>) -> TestService {
    service_with_limits(capacity_bytes, None)
}

fn request(identity: &str, payload: &[u8]) -> PreparedPublicationRequestV1<Vec<u8>> {
    PreparedPublicationRequestV1::new(
        PreparedPublicationIdentityV1::new(identity.as_bytes().to_vec()).expect("identity"),
        payload.to_vec(),
    )
}

// Independent V1 oracle: identity-index key + two token copies + two
// ordered-index slots + evidence state; active has a second identity copy,
// canonical key, payload, and one entry state. Do not call production cost
// helpers here: a shared formula bug must fail these assertions.
fn evidence_cost(identity: &str) -> u64 {
    u64::try_from(identity.len()).expect("test identity fits u64") + 128
}

fn live_cost(identity: &str, payload: &[u8]) -> u64 {
    evidence_cost(identity)
        + u64::try_from(identity.len()).expect("test identity fits u64")
        + 2 * u64::try_from(payload.len()).expect("test payload fits u64")
        + 16
}

fn assert_identity_evidence_invariant(service: &TestService) {
    let state = service.state.lock().expect("invariant state");
    let evidence_bytes: u64 = state.identity_index.values().map(|item| item.retained_bytes).sum();
    let active_bytes: u64 = state.entries.values().map(|item| item.active_bytes).sum();
    let aborted_bytes: u64 = state
        .identity_index
        .values()
        .filter(|item| matches!(&item.state, IdentityState::Aborted(_)))
        .map(|item| item.retained_bytes)
        .sum();
    assert_eq!(state.persistent_bytes, evidence_bytes);
    assert_eq!(state.charged_bytes, evidence_bytes + active_bytes);
    assert_eq!(state.identity_index.len(), state.entries.len() + state.aborted_tokens.len());
    assert_eq!(state.tombstone_bytes, aborted_bytes);
}

fn prepare_after_busy(
    service: &TestService,
    request: PreparedPublicationRequestV1<Vec<u8>>,
) -> Result<PreparedPublicationReceiptV1, PreparedPublicationErrorV1> {
    for _ in 0..10_000 {
        match service.prepare(request.clone()) {
            Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_))) => {
                thread::yield_now();
            }
            result => return result,
        }
    }
    panic!("prepared publication mutation lane failed to reopen")
}

#[cfg(feature = "core-live-accounting")]
mod accounting;
