use super::*;

impl<Payload, Mode> PreparedPublicationService<Payload, Mode>
where
    Payload: Clone + ByteSized + CanonicalKey,
{
    pub fn prepare(
        &self,
        request: PreparedPublicationRequestV1<Payload>,
    ) -> Result<PreparedPublicationReceiptV1, PreparedPublicationErrorV1> {
        let _permit =
            self.lifecycle.begin_mutation().map_err(PreparedPublicationErrorV1::Storage)?;
        // Caller-trait code (CanonicalKey/ByteSized) runs while the owned
        // payload is guarded, so a trait panic can never unwind through a
        // caller destructor (compound-hostile containment).
        let mut payload_in_flight = InFlight::new(request.payload);
        let payload_key = payload_in_flight.get().canonical_key();
        let token = derive_token(&request.identity, &payload_key)?;
        let state = self.lock_state()?;

        if let Some(identity_evidence) = state.identity_index.get(&request.identity) {
            return match &identity_evidence.state {
                IdentityState::Prepared(existing_token) => {
                    let existing = state.entries.get(existing_token).ok_or_else(|| {
                        PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                            "prepared identity index points to a missing token".to_string(),
                        ))
                    })?;
                    if existing.canonical_key == payload_key {
                        Ok(PreparedPublicationReceiptV1::new(
                            existing_token.clone(),
                            PrepareDispositionV1::Replayed,
                        ))
                    } else {
                        Err(PreparedPublicationErrorV1::IdentityConflict {
                            identity: request.identity,
                        })
                    }
                }
                IdentityState::Aborted(existing_token) if existing_token == &token => {
                    Err(PreparedPublicationErrorV1::AbortedToken { token })
                }
                IdentityState::Aborted(_) => {
                    Err(PreparedPublicationErrorV1::IdentityConflict { identity: request.identity })
                }
            };
        }

        if state.entries.contains_key(&token) || state.aborted_tokens.contains(&token) {
            return Err(PreparedPublicationErrorV1::TokenCollision { token });
        }

        // Every live prepared entry can be a GC root. Never admit a state
        // whose complete root set cannot be supplied to the bounded GC API,
        // including when unbudgeted accounting has no byte ceiling.
        if state.entries.len() >= MAX_REFERENCED_ROOT_TOKENS {
            return Err(PreparedPublicationErrorV1::Storage(MemoryError::capacity(format!(
                "prepared publication live entry count reached the GC root limit of {MAX_REFERENCED_ROOT_TOKENS}"
            ))));
        }

        let persistent_bytes = persistent_cost(request.identity.as_bytes().len())?;
        // Reserve the eventual tombstone while this entry is still live. The
        // identity index retains every admitted identity after abort/GC, so
        // counting only already-aborted entries can strand live payloads.
        let available_count =
            self.identity_evidence_count_capacity.saturating_sub(state.identity_index.len());
        let available_bytes =
            self.identity_evidence_bytes_capacity.saturating_sub(state.persistent_bytes);
        let next_persistent_bytes = match state.persistent_bytes.checked_add(persistent_bytes) {
            Some(next) if available_count > 0 && persistent_bytes <= available_bytes => next,
            _ => {
                return Err(PreparedPublicationErrorV1::IdentityEvidenceCapacity {
                    requested_count: 1,
                    available_count,
                    requested_bytes: persistent_bytes,
                    available_bytes,
                });
            }
        };
        // Only fresh tokens need measurement. Catch a client panic, release
        // the state guard, then resume unwinding so the intact state is not
        // poisoned; the in-flight guard retires the payload before that
        // resumed unwind drops it, so a compound hostile payload cannot
        // abort the process.
        let payload_bytes = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            payload_in_flight.get().byte_size()
        })) {
            Ok(bytes) => bytes,
            Err(payload) => {
                drop(state);
                std::panic::resume_unwind(payload);
            }
        };
        let active_bytes =
            active_cost(payload_bytes, request.identity.as_bytes().len(), payload_key.len())?;
        let requested_bytes = persistent_bytes.checked_add(active_bytes).ok_or_else(|| {
            PreparedPublicationErrorV1::Accounting(MemoryError::capacity(
                "prepared publication combined retained cost overflow",
            ))
        })?;
        let next_total = state
            .charged_bytes
            .checked_add(requested_bytes)
            .ok_or(PreparedPublicationErrorV1::Capacity { requested_bytes, available_bytes: 0 })?;
        if let Some(capacity_bytes) = self.capacity_bytes {
            if next_total > capacity_bytes {
                return Err(PreparedPublicationErrorV1::Capacity {
                    requested_bytes,
                    available_bytes: capacity_bytes.saturating_sub(state.charged_bytes),
                });
            }
        }

        drop(state);
        // The persistent lease is the identity/index/tombstone lifetime
        // component: a semantic metadata bucket that survives abort as the
        // tombstone. Only the active lease covers the payload component.
        let persistent_charge = self
            .accounting
            .charge(
                self.binding.class.clone(),
                self.binding.owner.clone(),
                MemoryContextKind::Request,
                ChargeKind::RetainedMetadata,
                persistent_bytes,
            )
            .map_err(PreparedPublicationErrorV1::Accounting)?;
        let active_charge = match self.accounting.charge(
            self.binding.class.clone(),
            self.binding.owner.clone(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            active_bytes,
        ) {
            Ok(charge) => charge,
            Err(error) => {
                drop(persistent_charge);
                return Err(PreparedPublicationErrorV1::Accounting(error));
            }
        };

        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => {
                drop(active_charge);
                drop(persistent_charge);
                return Err(error);
            }
        };
        let identity = request.identity;
        state.entries.insert(
            token.clone(),
            PreparedEntry {
                identity: identity.clone(),
                canonical_key: payload_key,
                payload: payload_in_flight.take(),
                active_bytes,
                active_charge,
            },
        );
        state.identity_index.insert(
            identity,
            IdentityEvidence {
                state: IdentityState::Prepared(token.clone()),
                retained_bytes: persistent_bytes,
                charge: persistent_charge,
            },
        );
        state.charged_bytes = next_total;
        state.persistent_bytes = next_persistent_bytes;
        Ok(PreparedPublicationReceiptV1::new(token, PrepareDispositionV1::Applied))
    }
}
