mod cases_01;
mod cases_02;
mod lifecycle_paging;

use super::*;
use crate::test_support::SingleTestCommand;
use crate::transcript_store::{
    TranscriptAppendIdentityV1, TranscriptAppendRequestV1, TranscriptRetentionPolicyV2,
};
use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
use std::sync::{Arc, Barrier};
use std::thread;

type Store = SimpleSessionTranscriptStore<String, String, String, TestUnbudgeted>;

fn cost(session_id: &str, rows: &[(&str, &str, bool)]) -> u64 {
    // Independent oracle: do not call the production cost calculators.
    (session_id.len() as u64 + 24 + 16)
        + rows
            .iter()
            .map(|(message_id, body, active)| {
                (2 * message_id.len() as u64 + 2 * 24 + 2 * 32 + 2 * 16)
                    + if *active {
                        body.len() as u64 + message_id.len() as u64 + 24 + 16
                    } else {
                        0
                    }
            })
            .sum::<u64>()
}

const ORDERED_INDEX_BYTES_ORACLE: u64 = 24;
const DIGEST_BYTES_ORACLE: u64 = 32;
const ENTRY_STATE_BYTES_ORACLE: u64 = 16;

fn expected_partition_costs(
    session_id: &str,
    message_id: &str,
    body: &str,
) -> (u64, u64, u64, u64) {
    let active = body.len() as u64
        + message_id.len() as u64
        + ORDERED_INDEX_BYTES_ORACLE
        + ENTRY_STATE_BYTES_ORACLE;
    let evidence = 2 * message_id.len() as u64
        + 2 * ORDERED_INDEX_BYTES_ORACLE
        + 2 * DIGEST_BYTES_ORACLE
        + 2 * ENTRY_STATE_BYTES_ORACLE;
    let session = session_id.len() as u64 + ORDERED_INDEX_BYTES_ORACLE + ENTRY_STATE_BYTES_ORACLE;
    let total = active + evidence + session;
    (active, evidence, session, total)
}

fn store() -> Store {
    store_with_policy(TranscriptRetentionPolicyV2::default())
}

fn store_with_policy(retention: TranscriptRetentionPolicyV2) -> Store {
    Store::new_test_unbudgeted(
        StoreBindingHandle::new(
            MemoryClass::SessionTranscript,
            "transcript-idempotency-tests".to_string(),
            None,
        ),
        retention,
    )
    .expect("transcript store nonce")
}

fn request(
    handle: SessionHandle,
    sequence: u64,
    message_id: &str,
    body: &str,
) -> TranscriptAppendRequestV1<SessionHandle, String, String> {
    TranscriptAppendRequestV1::new(
        TranscriptAppendIdentityV1::new(handle, sequence, message_id.to_string()),
        body.to_string(),
    )
}

#[cfg(feature = "core-live-accounting")]
mod accounting;
