//! Public transcript session lifecycle through the facade builder and trait.
//!
//! Run with:
//! `cargo run -p quanta-memory-platform-facade --features transcript --example transcript_session`

use quanta_memory_platform_core::{MemoryClass, MemoryError, MemoryPreset, StoreBindingHandle};
use quanta_memory_platform_facade::{
    MemoryPlatformBuilder, SessionTranscriptStore, TranscriptAppendDispositionV1,
    TranscriptAppendErrorV1, TranscriptAppendIdentityV1, TranscriptAppendRequestV1,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let platform = MemoryPlatformBuilder::new().with_preset(MemoryPreset::Development).build()?;
    let store = platform
        .transcript_store_builder::<String, String, String>()
        .bind(StoreBindingHandle::new(
            MemoryClass::SessionTranscript,
            "transcript-example".to_string(),
            None,
        ))
        .build()?;
    let transcript: &dyn SessionTranscriptStore<String, String, String> = &store;

    let old = transcript.open_session("caller-session".to_string())?;
    for sequence in 0..2 {
        assert_eq!(
            transcript.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(old, sequence, format!("message-{sequence}")),
                format!("body-{sequence}"),
            ))?,
            TranscriptAppendDispositionV1::Applied
        );
    }
    let first = transcript.read_page(&old, None, 1, u64::MAX)?;
    assert_eq!(first.records().len(), 1);
    let cursor = first.next_cursor().expect("one row remains");
    let second = transcript.read_page(&old, Some(&cursor), 1, u64::MAX)?;
    assert_eq!(second.records().len(), 1);
    assert!(second.next_cursor().is_none());

    transcript.retire_session(&old)?;
    let reopened = transcript.open_session("caller-session".to_string())?;
    assert_ne!(old, reopened);
    let late = transcript.append(TranscriptAppendRequestV1::new(
        TranscriptAppendIdentityV1::new(old, 2, "late-old".to_string()),
        "must not enter the reopened session".to_string(),
    ));
    assert!(matches!(late, Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(_)))));
    assert!(transcript.load(&reopened)?.is_empty());
    transcript.retire_session(&reopened)?;
    assert_eq!(platform.broker().ledger_snapshot()?.total_owner_bytes(), 0);

    println!("transcript_session_lifecycle=verified");
    Ok(())
}
