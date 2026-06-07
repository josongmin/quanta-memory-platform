use std::sync::Mutex;

use quanta_memory_platform_core::{
    ChargeKind, MemoryContextKind, MemoryError, MemoryResult, ReservationTicket,
    StoreBindingHandle, StoreStats,
};

use crate::shared::accounting::SharedAccountingSink;
use crate::shared::measure::ByteSized;
use crate::transcript_store::SessionTranscriptStore;

#[derive(Debug)]
struct TranscriptEntry<SessionId, Message> {
    session_id: SessionId,
    message: Message,
    charged_bytes: u64,
    /// Ledger receipt when the store is charged against a broker; refunded when
    /// the message is dropped by truncation.
    ticket: Option<ReservationTicket>,
}

#[derive(Debug)]
struct TranscriptState<SessionId, Message> {
    entries: Vec<TranscriptEntry<SessionId, Message>>,
    /// Sum of retained message bytes across all sessions.
    retained_bytes: u64,
}

/// Portable session transcript store.
///
/// Owns the count/bytes retention helper shape (playbook 9.2). Every append charges
/// the message's real byte size, and truncation subtracts the bytes of the dropped
/// messages from the retained-byte counter. The reported `estimated_bytes` therefore
/// always equals the exact sum of surviving message bytes; truncation can never
/// undercount retained bytes (negative path 5).
#[derive(Debug)]
pub struct SimpleSessionTranscriptStore<SessionId, Message> {
    binding: StoreBindingHandle,
    charger: Option<SharedAccountingSink>,
    state: Mutex<TranscriptState<SessionId, Message>>,
}

impl<SessionId, Message> SimpleSessionTranscriptStore<SessionId, Message> {
    pub fn new(binding: StoreBindingHandle) -> Self {
        Self {
            binding,
            charger: None,
            state: Mutex::new(TranscriptState { entries: Vec::new(), retained_bytes: 0 }),
        }
    }

    /// Charge this store's message bytes against a broker ledger (accounting
    /// SSOT). Without a sink the store self-accounts (non-budgeted mode).
    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }
}

impl<SessionId, Message> SimpleSessionTranscriptStore<SessionId, Message>
where
    SessionId: PartialEq,
    Message: Clone + ByteSized,
{
    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, TranscriptState<SessionId, Message>>, MemoryError> {
        self.state
            .lock()
            .map_err(|_| MemoryError::InvalidState("transcript store lock poisoned".to_string()))
    }

    /// Total retained transcript bytes across all sessions.
    pub fn retained_bytes(&self) -> Result<u64, MemoryError> {
        Ok(self.lock()?.retained_bytes)
    }

    /// Retained bytes for one session.
    pub fn session_bytes(&self, session_id: &SessionId) -> Result<u64, MemoryError> {
        Ok(self
            .lock()?
            .entries
            .iter()
            .filter(|entry| &entry.session_id == session_id)
            .map(|entry| entry.charged_bytes)
            .sum())
    }
}

impl<SessionId, Message> SessionTranscriptStore<SessionId, Message>
    for SimpleSessionTranscriptStore<SessionId, Message>
where
    SessionId: PartialEq,
    Message: Clone + ByteSized,
{
    fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn append(&self, session_id: SessionId, message: Message) -> MemoryResult<()> {
        let charged_bytes = message.byte_size();
        let mut state = self.lock()?;
        // Charge the ledger BEFORE recording the message so a cap denial fails
        // closed: nothing retained, counter untouched.
        let ticket = match &self.charger {
            Some(sink) => Some(sink.charge(
                self.binding.class.clone(),
                self.binding.owner.clone(),
                MemoryContextKind::Session,
                ChargeKind::RetainedPayload,
                charged_bytes,
            )?),
            None => None,
        };
        state.entries.push(TranscriptEntry { session_id, message, charged_bytes, ticket });
        state.retained_bytes = state.retained_bytes.saturating_add(charged_bytes);
        Ok(())
    }

    fn load(&self, session_id: &SessionId) -> Vec<Message> {
        let state = match self.lock() {
            Ok(state) => state,
            Err(_) => return Vec::new(),
        };
        state
            .entries
            .iter()
            .filter(|entry| &entry.session_id == session_id)
            .map(|entry| entry.message.clone())
            .collect()
    }

    fn truncate_to(&self, session_id: &SessionId, max_messages: usize) -> MemoryResult<()> {
        let mut state = self.lock()?;
        let mut kept = 0usize;
        let mut dropped_bytes = 0u64;
        let mut dropped_tickets: Vec<ReservationTicket> = Vec::new();
        let mut drop_indices = Vec::new();
        for (index, entry) in state.entries.iter().enumerate() {
            if &entry.session_id != session_id {
                continue;
            }
            kept += 1;
            if kept > max_messages {
                dropped_bytes = dropped_bytes.saturating_add(entry.charged_bytes);
                if let Some(ticket) = &entry.ticket {
                    dropped_tickets.push(ticket.clone());
                }
                drop_indices.push(index);
            }
        }
        if let Some(sink) = &self.charger {
            sink.refund_many(&dropped_tickets)?;
        }
        for index in drop_indices.into_iter().rev() {
            state.entries.remove(index);
        }
        state.retained_bytes = state.retained_bytes.saturating_sub(dropped_bytes);
        Ok(())
    }

    fn stats(&self) -> StoreStats {
        match self.lock() {
            Ok(state) => StoreStats {
                item_count: state.entries.len(),
                estimated_bytes: state.retained_bytes,
            },
            Err(_) => StoreStats::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};

    fn store() -> SimpleSessionTranscriptStore<String, String> {
        let binding =
            StoreBindingHandle::new(MemoryClass::SessionTranscript, "owner-tx".to_string(), None);
        SimpleSessionTranscriptStore::new(binding)
    }

    #[test]
    fn append_charges_real_message_bytes() {
        let store = store();
        store.append("s1".to_string(), "hello".to_string()).expect("append");
        store.append("s1".to_string(), "world!".to_string()).expect("append");
        // retained bytes == exact sum of message byte sizes (5 + 6).
        assert_eq!(store.retained_bytes().expect("retained"), 11);
        assert_eq!(store.session_bytes(&"s1".to_string()).expect("session"), 11);
        assert_eq!(store.stats(), StoreStats { item_count: 2, estimated_bytes: 11 });
    }

    #[test]
    fn truncate_releases_exactly_the_dropped_bytes_no_undercount() {
        let store = store();
        // Three messages of different sizes: 2, 8, 4 bytes.
        store.append("s".to_string(), "aa".to_string()).expect("a");
        store.append("s".to_string(), "bbbbbbbb".to_string()).expect("b");
        store.append("s".to_string(), "cccc".to_string()).expect("c");
        assert_eq!(store.retained_bytes().expect("before"), 2 + 8 + 4);

        // Truncate to keep only the first message.
        store.truncate_to(&"s".to_string(), 1).expect("truncate");

        // NEGATIVE-PATH 5: truncation must NOT undercount retained bytes. After
        // dropping the 8- and 4-byte messages, retained bytes must equal exactly
        // the surviving message's bytes, derived from SUT state (sum of survivors),
        // not a guessed constant.
        let survivors_bytes: u64 =
            store.load(&"s".to_string()).iter().map(|m| m.len() as u64).sum();
        assert_eq!(
            store.retained_bytes().expect("after truncate"),
            survivors_bytes,
            "retained bytes must equal the sum of surviving message bytes"
        );
        assert_eq!(store.retained_bytes().expect("after truncate"), 2);
        assert_eq!(store.stats().item_count, 1);
        assert_eq!(store.session_bytes(&"s".to_string()).expect("session"), 2);
    }

    #[test]
    fn truncate_one_session_does_not_touch_other_session_bytes() {
        let store = store();
        store.append("a".to_string(), "xxxx".to_string()).expect("a1");
        store.append("a".to_string(), "yyyy".to_string()).expect("a2");
        store.append("b".to_string(), "zz".to_string()).expect("b1");
        assert_eq!(store.retained_bytes().expect("before"), 4 + 4 + 2);

        store.truncate_to(&"a".to_string(), 0).expect("truncate a fully");
        // Session b is untouched; only a's 8 bytes are released.
        assert_eq!(store.session_bytes(&"a".to_string()).expect("a"), 0);
        assert_eq!(store.session_bytes(&"b".to_string()).expect("b"), 2);
        assert_eq!(store.retained_bytes().expect("after"), 2);
    }

    #[test]
    fn truncate_above_count_is_a_noop_on_bytes() {
        let store = store();
        store.append("s".to_string(), "keep".to_string()).expect("append");
        let before = store.retained_bytes().expect("before");
        store.truncate_to(&"s".to_string(), 100).expect("truncate above count");
        assert_eq!(store.retained_bytes().expect("after"), before);
        assert_eq!(store.stats().item_count, 1);
    }

    #[test]
    fn append_to_one_session_does_not_change_another_sessions_result_or_cost() {
        // scenario F: per-session isolation. Appending to session A must leave both
        // session B's observable messages AND its observable cost byte-for-byte
        // unchanged. Capture B's state BEFORE the A-append, assert it AFTER.
        let store = store();
        store.append("b".to_string(), "seed".to_string()).expect("seed b");
        let b_messages_before = store.load(&"b".to_string());
        let b_bytes_before = store.session_bytes(&"b".to_string()).expect("b bytes before");

        // Append to a DIFFERENT session.
        store.append("a".to_string(), "unrelated-message".to_string()).expect("append a");

        // Session B is untouched: same messages, same cost.
        assert_eq!(store.load(&"b".to_string()), b_messages_before, "B's messages must not change");
        assert_eq!(
            store.session_bytes(&"b".to_string()).expect("b bytes after"),
            b_bytes_before,
            "B's observable cost must not change when A is appended to"
        );
        // Global retained bytes account both sessions' charges, with no leakage.
        assert_eq!(
            store.retained_bytes().expect("global"),
            b_bytes_before + "unrelated-message".len() as u64,
        );
    }

    #[test]
    fn load_returns_only_requested_session_messages() {
        let store = store();
        store.append("a".to_string(), "m1".to_string()).expect("a");
        store.append("b".to_string(), "m2".to_string()).expect("b");
        assert_eq!(store.load(&"a".to_string()), vec!["m1".to_string()]);
        assert_eq!(store.load(&"missing".to_string()), Vec::<String>::new());
    }

    // ---- #3: a charged transcript store routes message bytes through the ledger ----
    #[cfg(feature = "core-live-accounting")]
    mod charged {
        use super::*;
        use crate::shared::accounting::SharedAccountingSink;
        use quanta_memory_platform_core::{
            EffectiveMemoryBudgetReport, ExecutionBudget, LiveProcessMemoryBroker,
            MemoryBudgetManifest, MemoryClassBudget, MemoryPreset, ReclaimPriority,
            ResidencyPolicy,
        };
        use std::sync::Arc;

        fn live_broker(class_hard: u64) -> LiveProcessMemoryBroker {
            let residency = ResidencyPolicy::new(10_000_000).with_class_budget(
                MemoryClass::SessionTranscript,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: class_hard,
                    hard_cap_bytes: class_hard,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            );
            let report = EffectiveMemoryBudgetReport::from_manifest(
                MemoryBudgetManifest::new(
                    MemoryPreset::Production,
                    ExecutionBudget::default(),
                    residency,
                ),
                vec!["test".to_string()],
            );
            LiveProcessMemoryBroker::from_report(report).expect("valid broker")
        }

        fn charged(
            broker: &LiveProcessMemoryBroker,
        ) -> SimpleSessionTranscriptStore<String, String> {
            let sink: SharedAccountingSink = Arc::new(broker.clone());
            let binding = StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "owner-tx".to_string(),
                None,
            );
            SimpleSessionTranscriptStore::new(binding).charged_against(sink)
        }

        #[test]
        fn append_and_truncate_track_the_ledger() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.append("s".to_string(), "aa".to_string()).expect("a"); // 2
            store.append("s".to_string(), "bbbbbbbb".to_string()).expect("b"); // 8
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                10,
                "ledger sees retained transcript bytes"
            );
            // Truncate drops the 8-byte message and must refund the ledger by 8.
            store.truncate_to(&"s".to_string(), 1).expect("truncate");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                2,
                "truncation refunds exactly the dropped bytes to the ledger"
            );
        }

        #[test]
        fn append_over_cap_is_denied_and_fails_closed() {
            let broker = live_broker(4); // SessionTranscript hard cap 4 bytes
            let store = charged(&broker);
            let denied = store.append("s".to_string(), "toolong".to_string()); // 7 > 4
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "over-cap append must be a typed capacity error, got {denied:?}"
            );
            assert_eq!(store.stats().item_count, 0, "denied append stores nothing");
            assert_eq!(store.retained_bytes().expect("retained"), 0);
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
        }

        #[test]
        fn charged_append_isolates_other_sessions() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.append("b".to_string(), "seed".to_string()).expect("seed b");
            let b_messages_before = store.load(&"b".to_string());
            let b_bytes_before = store.session_bytes(&"b".to_string()).expect("b bytes before");
            let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();

            store.append("a".to_string(), "unrelated-message".to_string()).expect("append a");

            assert_eq!(store.load(&"b".to_string()), b_messages_before);
            assert_eq!(
                store.session_bytes(&"b".to_string()).expect("b bytes after"),
                b_bytes_before
            );
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                ledger_before + "unrelated-message".len() as u64
            );
        }

        #[test]
        fn truncate_during_rebuild_is_fail_closed() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.append("s".to_string(), "aa".to_string()).expect("a");
            store.append("s".to_string(), "bbbbbbbb".to_string()).expect("b");
            let before_messages = store.load(&"s".to_string());
            let before_session_bytes =
                store.session_bytes(&"s".to_string()).expect("session bytes");
            broker.begin_rebuild().expect("begin rebuild");

            let denied = store.truncate_to(&"s".to_string(), 1);
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "truncate during rebuild must fail closed, got {denied:?}"
            );
            assert_eq!(store.load(&"s".to_string()), before_messages);
            assert_eq!(store.session_bytes(&"s".to_string()).expect("after"), before_session_bytes);
            assert_eq!(store.retained_bytes().expect("retained"), before_session_bytes);
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                before_session_bytes
            );
        }
    }
}
