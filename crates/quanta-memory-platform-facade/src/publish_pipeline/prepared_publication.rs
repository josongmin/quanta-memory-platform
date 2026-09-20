use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard, PoisonError};

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, MemoryContextKind, MemoryError,
    StoreBindingHandle,
};

#[cfg(feature = "unbudgeted-stores")]
use crate::shared::accounting::Unbudgeted;
use crate::shared::accounting::{Budgeted, StoreAccounting};
use crate::shared::lifecycle::{retire_client_value, BeginClose, DropTransition, StoreLifecycle};
use crate::shared::measure::{ByteSized, CanonicalKey};

const TOKEN_DOMAIN: &[u8] = b"qmp.prepared-publication.v1\0";
const DEFAULT_TOMBSTONE_COUNT_CAPACITY: usize = 4_096;
const DEFAULT_TOMBSTONE_BYTES_CAPACITY: u64 = 4 * 1024 * 1024;
const TOKEN_BYTES: u64 = 32;

/// Caller-owned stable identity for one logical publication attempt.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreparedPublicationIdentityV1(Vec<u8>);

impl PreparedPublicationIdentityV1 {
    pub fn new(bytes: Vec<u8>) -> Result<Self, PreparedPublicationErrorV1> {
        if bytes.is_empty() {
            return Err(PreparedPublicationErrorV1::InvalidIdentity {
                reason: "prepared publication identity must not be empty".to_string(),
            });
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Stable, opaque content-and-identity token suitable for serialization in a
/// caller-owned canonical root.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreparedPublicationTokenV1([u8; 32]);

impl PreparedPublicationTokenV1 {
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for PreparedPublicationTokenV1 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "PreparedPublicationTokenV1(")?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(")")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPublicationRequestV1<Payload> {
    identity: PreparedPublicationIdentityV1,
    payload: Payload,
}

impl<Payload> PreparedPublicationRequestV1<Payload> {
    #[must_use]
    pub fn new(identity: PreparedPublicationIdentityV1, payload: Payload) -> Self {
        Self { identity, payload }
    }

    #[must_use]
    pub fn identity(&self) -> &PreparedPublicationIdentityV1 {
        &self.identity
    }

    #[must_use]
    pub fn payload(&self) -> &Payload {
        &self.payload
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareDispositionV1 {
    Applied,
    Replayed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortDispositionV1 {
    Aborted,
    Replayed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPublicationReceiptV1 {
    token: PreparedPublicationTokenV1,
    disposition: PrepareDispositionV1,
}

impl PreparedPublicationReceiptV1 {
    fn new(token: PreparedPublicationTokenV1, disposition: PrepareDispositionV1) -> Self {
        Self { token, disposition }
    }

    #[must_use]
    pub fn token(&self) -> &PreparedPublicationTokenV1 {
        &self.token
    }

    #[must_use]
    pub fn disposition(&self) -> PrepareDispositionV1 {
        self.disposition
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparedPublicationStatsV1 {
    pub prepared_count: usize,
    pub charged_bytes: u64,
    pub tombstone_count: usize,
    pub tombstone_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedPublicationErrorV1 {
    InvalidIdentity {
        reason: String,
    },
    Capacity {
        requested_bytes: u64,
        available_bytes: u64,
    },
    TombstoneCapacity {
        requested_count: usize,
        available_count: usize,
        requested_bytes: u64,
        available_bytes: u64,
    },
    IdentityConflict {
        identity: PreparedPublicationIdentityV1,
    },
    TokenCollision {
        token: PreparedPublicationTokenV1,
    },
    UnknownToken {
        token: PreparedPublicationTokenV1,
    },
    AbortedToken {
        token: PreparedPublicationTokenV1,
    },
    Accounting(MemoryError),
    Storage(MemoryError),
}

impl Display for PreparedPublicationErrorV1 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidIdentity { reason } => write!(f, "invalid identity: {reason}"),
            Self::Capacity { requested_bytes, available_bytes } => write!(
                f,
                "prepared publication capacity exceeded: requested {requested_bytes}, available {available_bytes}"
            ),
            Self::TombstoneCapacity {
                requested_count,
                available_count,
                requested_bytes,
                available_bytes,
            } => write!(
                f,
                "prepared publication tombstone capacity exceeded: requested {requested_count} entries/{requested_bytes} bytes, available {available_count} entries/{available_bytes} bytes"
            ),
            Self::IdentityConflict { identity } => {
                write!(f, "prepared publication identity conflict: {identity:?}")
            }
            Self::TokenCollision { token } => write!(f, "prepared publication token collision: {token:?}"),
            Self::UnknownToken { token } => write!(f, "unknown prepared publication token: {token:?}"),
            Self::AbortedToken { token } => write!(f, "prepared publication token was aborted: {token:?}"),
            Self::Accounting(error) => write!(f, "prepared publication accounting error: {error}"),
            Self::Storage(error) => write!(f, "prepared publication storage error: {error}"),
        }
    }
}

impl std::error::Error for PreparedPublicationErrorV1 {}

#[derive(Debug)]
struct PreparedEntry<Payload> {
    identity: PreparedPublicationIdentityV1,
    canonical_key: Vec<u8>,
    payload: Payload,
    charged_bytes: u64,
    /// Single-owner accounting authority for this exact retained payload.
    charge: Option<ManagedCharge>,
}

fn retire_entry<Payload>(entry: PreparedEntry<Payload>) {
    let PreparedEntry { payload, charge, .. } = entry;
    drop(charge);
    retire_client_value(payload);
}

fn retire_entries<Payload>(entries: BTreeMap<PreparedPublicationTokenV1, PreparedEntry<Payload>>) {
    for (_, entry) in entries {
        retire_entry(entry);
    }
}

#[derive(Clone, Debug)]
enum IdentityState {
    Prepared(PreparedPublicationTokenV1),
    Aborted(PreparedPublicationTokenV1),
}

#[derive(Debug)]
struct PreparedState<Payload> {
    entries: BTreeMap<PreparedPublicationTokenV1, PreparedEntry<Payload>>,
    identity_index: BTreeMap<PreparedPublicationIdentityV1, IdentityState>,
    aborted_tokens: BTreeSet<PreparedPublicationTokenV1>,
    charged_bytes: u64,
    tombstone_bytes: u64,
}

impl<Payload> PreparedState<Payload> {
    fn empty() -> Self {
        Self {
            entries: BTreeMap::new(),
            identity_index: BTreeMap::new(),
            aborted_tokens: BTreeSet::new(),
            charged_bytes: 0,
            tombstone_bytes: 0,
        }
    }
}

/// In-memory prepared-publication owner. It proves process-lifetime atomicity
/// and idempotency only; it does not claim restart durability or activation
/// ownership. `Mode` is mandatory and construction fixes the accounting policy:
///
/// ```compile_fail
/// use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
/// use quanta_memory_platform_facade::{Budgeted, PreparedPublicationService};
/// let binding = StoreBindingHandle::new(MemoryClass::ManifestResidency, "owner".into(), None);
/// let _ = PreparedPublicationService::<Vec<u8>, Budgeted>::new(binding, None);
/// ```
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{Budgeted, PreparedPublicationService};
/// fn late_attach(service: PreparedPublicationService<Vec<u8>, Budgeted>) {
///     let _ = service.charged_against;
/// }
/// ```
#[derive(Debug)]
pub struct PreparedPublicationService<Payload, Mode> {
    binding: StoreBindingHandle,
    capacity_bytes: Option<u64>,
    tombstone_count_capacity: usize,
    tombstone_bytes_capacity: u64,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    state: Mutex<PreparedState<Payload>>,
    _mode: PhantomData<fn() -> Mode>,
}

impl<Payload> PreparedPublicationService<Payload, Budgeted> {
    pub(crate) fn new_budgeted(
        binding: StoreBindingHandle,
        capacity_bytes: Option<u64>,
        tombstone_limits: Option<(usize, u64)>,
        broker: LiveProcessMemoryBroker,
    ) -> Self {
        Self::with_accounting(
            binding,
            capacity_bytes,
            tombstone_limits,
            StoreAccounting::Budgeted(broker),
        )
    }
}

#[cfg(feature = "unbudgeted-stores")]
impl<Payload> PreparedPublicationService<Payload, Unbudgeted> {
    pub(crate) fn new_unbudgeted(
        binding: StoreBindingHandle,
        capacity_bytes: Option<u64>,
        tombstone_limits: Option<(usize, u64)>,
    ) -> Self {
        Self::with_accounting(
            binding,
            capacity_bytes,
            tombstone_limits,
            StoreAccounting::Unbudgeted,
        )
    }
}

impl<Payload, Mode> PreparedPublicationService<Payload, Mode> {
    fn with_accounting(
        binding: StoreBindingHandle,
        capacity_bytes: Option<u64>,
        tombstone_limits: Option<(usize, u64)>,
        accounting: StoreAccounting,
    ) -> Self {
        let (tombstone_count_capacity, tombstone_bytes_capacity) = tombstone_limits
            .unwrap_or((DEFAULT_TOMBSTONE_COUNT_CAPACITY, DEFAULT_TOMBSTONE_BYTES_CAPACITY));
        Self {
            binding,
            capacity_bytes,
            tombstone_count_capacity,
            tombstone_bytes_capacity,
            accounting,
            lifecycle: StoreLifecycle::new("prepared publication service"),
            state: Mutex::new(PreparedState::empty()),
            _mode: PhantomData,
        }
    }

    #[must_use]
    pub fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn lock_state(
        &self,
    ) -> Result<MutexGuard<'_, PreparedState<Payload>>, PreparedPublicationErrorV1> {
        self.lifecycle.try_lock_state(&self.state).map_err(PreparedPublicationErrorV1::Storage)
    }

    /// Readers never observe entries while their accounting authority is in an
    /// out-of-lock release plan.
    fn lock_read(
        &self,
    ) -> Result<MutexGuard<'_, PreparedState<Payload>>, PreparedPublicationErrorV1> {
        self.lifecycle.ensure_readable().map_err(PreparedPublicationErrorV1::Storage)?;
        let state = self.lock_state()?;
        self.lifecycle.ensure_readable().map_err(PreparedPublicationErrorV1::Storage)?;
        Ok(state)
    }

    fn require_charge_shape(
        &self,
        entry: &PreparedEntry<Payload>,
        operation: &str,
    ) -> Result<(), PreparedPublicationErrorV1> {
        if self.accounting.broker().is_some() == entry.charge.is_some() {
            Ok(())
        } else {
            Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(format!(
                "prepared publication {operation} encountered mismatched accounting authority"
            ))))
        }
    }

    /// Release every live payload charge and enter the terminal closed state.
    /// A failed release restores the exact preimage and reopens the service.
    pub fn close(&self) -> Result<(), PreparedPublicationErrorV1> {
        let close =
            match self.lifecycle.begin_close().map_err(PreparedPublicationErrorV1::Storage)? {
                BeginClose::AlreadyClosed => return Ok(()),
                BeginClose::Acquired(close) => close,
            };

        let (tokens, mut charges) = {
            let mut state = match self.lock_state() {
                Ok(state) => state,
                Err(error) => {
                    close.rollback();
                    return Err(error);
                }
            };
            for entry in state.entries.values() {
                if let Err(error) = self.require_charge_shape(entry, "close") {
                    close.rollback();
                    return Err(error);
                }
            }
            let mut tokens = Vec::new();
            let mut charges = Vec::new();
            for (token, entry) in &mut state.entries {
                if let Some(charge) = entry.charge.take() {
                    tokens.push(token.clone());
                    charges.push(charge);
                }
            }
            (tokens, charges)
        };

        if let Err(error) = self.accounting.release_many(&mut charges) {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            for (token, charge) in tokens.into_iter().zip(charges) {
                if let Some(entry) = state.entries.get_mut(&token) {
                    entry.charge = Some(charge);
                }
            }
            close.rollback();
            return Err(PreparedPublicationErrorV1::Accounting(error));
        }

        let old_state = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::replace(&mut *state, PreparedState::empty())
        };
        close.commit();
        retire_entries(old_state.entries);
        Ok(())
    }
}

impl<Payload, Mode> Drop for PreparedPublicationService<Payload, Mode> {
    fn drop(&mut self) {
        if self.lifecycle.transition_to_closed_for_drop() == DropTransition::AlreadyClosed {
            return;
        }
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        let entries = std::mem::take(&mut state.entries);
        state.charged_bytes = 0;
        retire_entries(entries);
    }
}

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
        let payload_key = request.payload.canonical_key();
        let token = derive_token(&request.identity, &payload_key);
        let charged_bytes = request.payload.byte_size();
        let state = self.lock_state()?;

        if let Some(identity_state) = state.identity_index.get(&request.identity) {
            return match identity_state {
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

        let next_total = state.charged_bytes.checked_add(charged_bytes).ok_or(
            PreparedPublicationErrorV1::Capacity {
                requested_bytes: charged_bytes,
                available_bytes: 0,
            },
        )?;
        if let Some(capacity_bytes) = self.capacity_bytes {
            if next_total > capacity_bytes {
                return Err(PreparedPublicationErrorV1::Capacity {
                    requested_bytes: charged_bytes,
                    available_bytes: capacity_bytes.saturating_sub(state.charged_bytes),
                });
            }
        }

        drop(state);
        let charge = self
            .accounting
            .charge(
                self.binding.class.clone(),
                self.binding.owner.clone(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                charged_bytes,
            )
            .map_err(PreparedPublicationErrorV1::Accounting)?;

        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => {
                drop(charge);
                return Err(error);
            }
        };
        let identity = request.identity;
        state.entries.insert(
            token.clone(),
            PreparedEntry {
                identity: identity.clone(),
                canonical_key: payload_key,
                payload: request.payload,
                charged_bytes,
                charge,
            },
        );
        state.identity_index.insert(identity, IdentityState::Prepared(token.clone()));
        state.charged_bytes = next_total;
        Ok(PreparedPublicationReceiptV1::new(token, PrepareDispositionV1::Applied))
    }

    pub fn resolve(
        &self,
        token: &PreparedPublicationTokenV1,
    ) -> Result<Payload, PreparedPublicationErrorV1> {
        let state = self.lock_read()?;
        if let Some(entry) = state.entries.get(token) {
            return Ok(entry.payload.clone());
        }
        if state.aborted_tokens.contains(token) {
            return Err(PreparedPublicationErrorV1::AbortedToken { token: token.clone() });
        }
        Err(PreparedPublicationErrorV1::UnknownToken { token: token.clone() })
    }

    pub fn abort(
        &self,
        token: &PreparedPublicationTokenV1,
    ) -> Result<AbortDispositionV1, PreparedPublicationErrorV1> {
        let _permit =
            self.lifecycle.begin_mutation().map_err(PreparedPublicationErrorV1::Storage)?;
        let (next_total, next_tombstone_bytes, mut charge) = {
            let mut state = self.lock_state()?;
            if state.aborted_tokens.contains(token) {
                return Ok(AbortDispositionV1::Replayed);
            }
            let entry = state
                .entries
                .get(token)
                .ok_or_else(|| PreparedPublicationErrorV1::UnknownToken { token: token.clone() })?;
            self.require_charge_shape(entry, "abort")?;
            let tombstone_bytes = self.ensure_tombstone_capacity(&state, [entry])?;
            let next_total =
                state.charged_bytes.checked_sub(entry.charged_bytes).ok_or_else(|| {
                    PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                        "prepared publication retained-byte counter underflow".to_string(),
                    ))
                })?;
            let next_tombstone_bytes =
                state.tombstone_bytes.checked_add(tombstone_bytes).ok_or_else(|| {
                    PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                        "prepared publication tombstone-byte counter overflow".to_string(),
                    ))
                })?;
            let charge = state.entries.get_mut(token).and_then(|entry| entry.charge.take());
            (next_total, next_tombstone_bytes, charge)
        };
        if let Some(active) = charge.as_mut() {
            if let Err(error) = self.accounting.release_many(std::slice::from_mut(active)) {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.entries.get_mut(token).expect("mutation lane owns entry").charge = charge;
                return Err(PreparedPublicationErrorV1::Accounting(error));
            }
        }
        let entry = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = state.entries.remove(token).expect("mutation lane owns entry");
            state.charged_bytes = next_total;
            state
                .identity_index
                .insert(entry.identity.clone(), IdentityState::Aborted(token.clone()));
            state.aborted_tokens.insert(token.clone());
            state.tombstone_bytes = next_tombstone_bytes;
            entry
        };
        retire_entry(entry);
        Ok(AbortDispositionV1::Aborted)
    }

    /// Enumerate prepared entries not referenced by the caller's complete
    /// canonical-root snapshot.
    ///
    /// QMP validates the supplied references but does not own activation state.
    /// Callers must therefore stabilize their root snapshot for the duration of
    /// this call and supply every referenced token; a missing or aborted
    /// reference fails closed.
    pub fn orphan_tokens(
        &self,
        referenced_tokens: &[PreparedPublicationTokenV1],
    ) -> Result<Vec<PreparedPublicationTokenV1>, PreparedPublicationErrorV1> {
        let state = self.lock_read()?;
        classify_orphan_tokens(&state, referenced_tokens)
    }

    /// Collect every prepared entry absent from the caller's complete
    /// canonical-root snapshot.
    ///
    /// Reference validation and the complete refund/removal batch are serialized
    /// by the lifecycle mutation lane. Broker accounting runs outside the state
    /// mutex. The caller must serialize root activation against this call; QMP
    /// never infers or owns activation/publication itself.
    pub fn gc_orphans(
        &self,
        referenced_tokens: &[PreparedPublicationTokenV1],
    ) -> Result<usize, PreparedPublicationErrorV1> {
        let _permit =
            self.lifecycle.begin_mutation().map_err(PreparedPublicationErrorV1::Storage)?;
        let (prepared_tokens, charged_tokens, next_total, next_tombstone_bytes, mut charges) = {
            let mut state = self.lock_state()?;
            let prepared_tokens = classify_orphan_tokens(&state, referenced_tokens)?;
            let tombstone_entries = prepared_tokens
                .iter()
                .map(|token| state.entries.get(token).expect("classified token is live"))
                .collect::<Vec<_>>();
            for entry in &tombstone_entries {
                self.require_charge_shape(entry, "orphan GC")?;
            }
            let tombstone_bytes =
                self.ensure_tombstone_capacity(&state, tombstone_entries.iter().copied())?;
            let reclaimed_bytes = tombstone_entries.iter().try_fold(0u64, |total, entry| {
                total.checked_add(entry.charged_bytes).ok_or_else(|| {
                    PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                        "prepared publication reclaimed-byte counter overflow".to_string(),
                    ))
                })
            })?;
            let next_total = state.charged_bytes.checked_sub(reclaimed_bytes).ok_or_else(|| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "prepared publication retained-byte counter underflow".to_string(),
                ))
            })?;
            let next_tombstone_bytes =
                state.tombstone_bytes.checked_add(tombstone_bytes).ok_or_else(|| {
                    PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                        "prepared publication tombstone-byte counter overflow".to_string(),
                    ))
                })?;
            let mut charged_tokens = Vec::new();
            let mut charges = Vec::new();
            for token in &prepared_tokens {
                if let Some(charge) =
                    state.entries.get_mut(token).expect("classified token is live").charge.take()
                {
                    charged_tokens.push(token.clone());
                    charges.push(charge);
                }
            }
            (prepared_tokens, charged_tokens, next_total, next_tombstone_bytes, charges)
        };
        if let Err(error) = self.accounting.release_many(&mut charges) {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            for (token, charge) in charged_tokens.into_iter().zip(charges) {
                state.entries.get_mut(&token).expect("mutation lane owns entry").charge =
                    Some(charge);
            }
            return Err(PreparedPublicationErrorV1::Accounting(error));
        }
        let removed = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let mut removed = Vec::with_capacity(prepared_tokens.len());
            for token in &prepared_tokens {
                let entry = state.entries.remove(token).expect("mutation lane owns entry");
                state
                    .identity_index
                    .insert(entry.identity.clone(), IdentityState::Aborted(token.clone()));
                state.aborted_tokens.insert(token.clone());
                removed.push(entry);
            }
            state.charged_bytes = next_total;
            state.tombstone_bytes = next_tombstone_bytes;
            removed
        };
        for entry in removed {
            retire_entry(entry);
        }
        Ok(prepared_tokens.len())
    }

    pub fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1> {
        let state = self.lock_read()?;
        Ok(PreparedPublicationStatsV1 {
            prepared_count: state.entries.len(),
            charged_bytes: state.charged_bytes,
            tombstone_count: state.aborted_tokens.len(),
            tombstone_bytes: state.tombstone_bytes,
        })
    }

    fn ensure_tombstone_capacity<'a>(
        &self,
        state: &PreparedState<Payload>,
        entries: impl IntoIterator<Item = &'a PreparedEntry<Payload>>,
    ) -> Result<u64, PreparedPublicationErrorV1>
    where
        Payload: 'a,
    {
        let mut requested_count = 0usize;
        let mut requested_bytes = 0u64;
        for entry in entries {
            requested_count = requested_count.checked_add(1).ok_or_else(|| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "prepared publication tombstone-count overflow".to_string(),
                ))
            })?;
            let identity_bytes = u64::try_from(entry.identity.as_bytes().len()).map_err(|_| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "prepared publication identity length does not fit u64".to_string(),
                ))
            })?;
            requested_bytes = requested_bytes
                .checked_add(identity_bytes)
                .and_then(|bytes| bytes.checked_add(TOKEN_BYTES))
                .ok_or_else(|| {
                    PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                        "prepared publication tombstone-byte counter overflow".to_string(),
                    ))
                })?;
        }

        let available_count =
            self.tombstone_count_capacity.saturating_sub(state.aborted_tokens.len());
        let available_bytes = self.tombstone_bytes_capacity.saturating_sub(state.tombstone_bytes);
        if requested_count > available_count || requested_bytes > available_bytes {
            return Err(PreparedPublicationErrorV1::TombstoneCapacity {
                requested_count,
                available_count,
                requested_bytes,
                available_bytes,
            });
        }
        Ok(requested_bytes)
    }
}

fn classify_orphan_tokens<Payload>(
    state: &PreparedState<Payload>,
    referenced_tokens: &[PreparedPublicationTokenV1],
) -> Result<Vec<PreparedPublicationTokenV1>, PreparedPublicationErrorV1> {
    let referenced: BTreeSet<_> = referenced_tokens.iter().cloned().collect();
    for token in &referenced {
        if state.aborted_tokens.contains(token) {
            return Err(PreparedPublicationErrorV1::AbortedToken { token: token.clone() });
        }
        if !state.entries.contains_key(token) {
            return Err(PreparedPublicationErrorV1::UnknownToken { token: token.clone() });
        }
    }
    Ok(state.entries.keys().filter(|token| !referenced.contains(*token)).cloned().collect())
}

fn derive_token(
    identity: &PreparedPublicationIdentityV1,
    payload_key: &[u8],
) -> PreparedPublicationTokenV1 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(TOKEN_DOMAIN);
    hasher.update(&(identity.0.len() as u64).to_le_bytes());
    hasher.update(&identity.0);
    hasher.update(&(payload_key.len() as u64).to_le_bytes());
    hasher.update(payload_key);
    PreparedPublicationTokenV1(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
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

    fn service_with_limits(
        capacity_bytes: Option<u64>,
        limits: Option<(usize, u64)>,
    ) -> TestService {
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

    #[test]
    fn prepare_capacity_failure_leaves_all_state_unchanged() {
        let service = service(Some(3));
        let err = service.prepare(request("id-1", b"four")).expect_err("capacity denial");
        assert!(matches!(err, PreparedPublicationErrorV1::Capacity { .. }));
        assert_eq!(service.stats().expect("stats"), PreparedPublicationStatsV1::default());
        assert!(service.orphan_tokens(&[]).expect("orphans").is_empty());
    }

    #[test]
    fn prepared_payload_is_only_reachable_by_exact_token() {
        let service = service(None);
        let receipt = service.prepare(request("id-1", b"payload")).expect("prepare");
        assert_eq!(receipt.disposition(), PrepareDispositionV1::Applied);
        assert_eq!(service.resolve(receipt.token()).expect("resolve"), b"payload".to_vec());
        let forged = PreparedPublicationTokenV1::from_bytes([0xA5; 32]);
        assert!(matches!(
            service.resolve(&forged),
            Err(PreparedPublicationErrorV1::UnknownToken { .. })
        ));
        assert_eq!(service.stats().expect("stats").prepared_count, 1);
    }

    #[test]
    fn identical_prepare_replays_same_token_without_second_charge() {
        let service = service(None);
        let first = service.prepare(request("id-1", b"payload")).expect("first");
        let replay = service.prepare(request("id-1", b"payload")).expect("replay");
        assert_eq!(first.token(), replay.token());
        assert_eq!(first.disposition(), PrepareDispositionV1::Applied);
        assert_eq!(replay.disposition(), PrepareDispositionV1::Replayed);
        assert_eq!(service.stats().expect("stats").charged_bytes, 7);
    }

    #[test]
    fn identity_payload_drift_is_typed_conflict() {
        let service = service(None);
        let original = service.prepare(request("id-1", b"old")).expect("original");
        let err = service.prepare(request("id-1", b"new")).expect_err("payload drift");
        assert!(matches!(err, PreparedPublicationErrorV1::IdentityConflict { .. }));
        assert_eq!(service.resolve(original.token()).expect("original intact"), b"old".to_vec());
        assert_eq!(service.stats().expect("stats").charged_bytes, 3);
    }

    #[test]
    fn abort_releases_exact_payload_and_is_retry_safe() {
        let service = service(None);
        let receipt = service.prepare(request("id-1", b"payload")).expect("prepare");
        assert_eq!(service.abort(receipt.token()).expect("abort"), AbortDispositionV1::Aborted);
        assert_eq!(
            service.abort(receipt.token()).expect("abort replay"),
            AbortDispositionV1::Replayed
        );
        assert!(matches!(
            service.resolve(receipt.token()),
            Err(PreparedPublicationErrorV1::AbortedToken { .. })
        ));
        assert_eq!(
            service.stats().expect("stats"),
            PreparedPublicationStatsV1 {
                prepared_count: 0,
                charged_bytes: 0,
                tombstone_count: 1,
                tombstone_bytes: 36,
            }
        );
    }

    #[test]
    fn concurrent_identical_prepare_has_one_applied_winner() {
        const N: usize = 8;
        let service = Arc::new(service(None));
        let barrier = Arc::new(Barrier::new(N));
        let handles: Vec<_> = (0..N)
            .map(|_| {
                let service = Arc::clone(&service);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    prepare_after_busy(&service, request("same", b"payload")).expect("prepare")
                })
            })
            .collect();
        let receipts: Vec<_> = handles.into_iter().map(|h| h.join().expect("join")).collect();
        assert_eq!(
            receipts.iter().filter(|r| r.disposition() == PrepareDispositionV1::Applied).count(),
            1
        );
        assert_eq!(
            receipts.iter().filter(|r| r.disposition() == PrepareDispositionV1::Replayed).count(),
            N - 1
        );
        assert!(receipts.iter().all(|r| r.token() == receipts[0].token()));
        assert_eq!(service.stats().expect("stats").charged_bytes, 7);
    }

    #[test]
    fn concurrent_conflicting_prepare_has_one_winner() {
        let service = Arc::new(service(None));
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = [b"left".to_vec(), b"right".to_vec()]
            .into_iter()
            .map(|payload| {
                let service = Arc::clone(&service);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    prepare_after_busy(&service, request("same", &payload))
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().expect("join")).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(PreparedPublicationErrorV1::IdentityConflict { .. })))
                .count(),
            1
        );
        assert_eq!(service.stats().expect("stats").prepared_count, 1);
    }

    #[test]
    fn root_referenced_token_is_never_enumerated_or_collected_as_an_orphan() {
        let service = service(None);
        let first = service.prepare(request("id-1", b"one")).expect("first");
        let second = service.prepare(request("id-2", b"two")).expect("second");
        let referenced = [second.token().clone()];
        assert_eq!(
            service.orphan_tokens(&referenced).expect("orphans"),
            vec![first.token().clone()]
        );
        assert_eq!(service.gc_orphans(&referenced).expect("gc"), 1);
        assert!(matches!(
            service.resolve(first.token()),
            Err(PreparedPublicationErrorV1::AbortedToken { .. })
        ));
        assert_eq!(service.resolve(second.token()).expect("survivor"), b"two".to_vec());
        assert_eq!(service.stats().expect("stats").prepared_count, 1);
    }

    #[test]
    fn orphan_gc_with_unknown_root_reference_is_batch_atomic() {
        let service = service(None);
        let first = service.prepare(request("id-1", b"one")).expect("first");
        let second = service.prepare(request("id-2", b"two")).expect("second");
        let forged = PreparedPublicationTokenV1::from_bytes([0xA5; 32]);

        let error = service
            .gc_orphans(&[forged])
            .expect_err("the root snapshot must reject an unknown token");
        assert!(matches!(error, PreparedPublicationErrorV1::UnknownToken { .. }));
        assert_eq!(service.resolve(first.token()).expect("first intact"), b"one".to_vec());
        assert_eq!(service.resolve(second.token()).expect("second intact"), b"two".to_vec());
        assert_eq!(
            service.stats().expect("stats"),
            PreparedPublicationStatsV1 {
                prepared_count: 2,
                charged_bytes: 6,
                ..PreparedPublicationStatsV1::default()
            }
        );
    }

    #[test]
    fn aborted_root_reference_fails_closed_without_collecting_other_entries() {
        let service = service(None);
        let aborted = service.prepare(request("id-1", b"one")).expect("aborted");
        let survivor = service.prepare(request("id-2", b"two")).expect("survivor");
        service.abort(aborted.token()).expect("abort");

        let error = service
            .gc_orphans(&[aborted.token().clone()])
            .expect_err("an aborted canonical-root reference must fail closed");
        assert!(matches!(error, PreparedPublicationErrorV1::AbortedToken { .. }));
        assert_eq!(service.resolve(survivor.token()).expect("survivor intact"), b"two".to_vec());
        assert_eq!(
            service.stats().expect("stats"),
            PreparedPublicationStatsV1 {
                prepared_count: 1,
                charged_bytes: 3,
                tombstone_count: 1,
                tombstone_bytes: 36,
            }
        );
    }

    #[test]
    fn tombstone_limits_bound_n_cycle_churn_and_fail_closed_at_saturation() {
        const MAX_TOMBSTONES: usize = 4;
        const TOMBSTONE_BYTES_PER_CYCLE: u64 = 36;
        let service = service_with_limits(
            None,
            Some((MAX_TOMBSTONES, MAX_TOMBSTONES as u64 * TOMBSTONE_BYTES_PER_CYCLE)),
        );

        for cycle in 0..MAX_TOMBSTONES {
            let identity = format!("id-{cycle}");
            service.prepare(request(&identity, b"x")).expect("prepare");
            assert_eq!(service.gc_orphans(&[]).expect("gc"), 1);
        }
        assert_eq!(
            service.stats().expect("bounded stats"),
            PreparedPublicationStatsV1 {
                prepared_count: 0,
                charged_bytes: 0,
                tombstone_count: MAX_TOMBSTONES,
                tombstone_bytes: MAX_TOMBSTONES as u64 * TOMBSTONE_BYTES_PER_CYCLE,
            }
        );

        let saturated = service.prepare(request("id-4", b"x")).expect("prepare at limit");
        let error = service.gc_orphans(&[]).expect_err("GC must fail before dropping payload");
        assert!(matches!(
            error,
            PreparedPublicationErrorV1::TombstoneCapacity {
                requested_count: 1,
                available_count: 0,
                requested_bytes: TOMBSTONE_BYTES_PER_CYCLE,
                available_bytes: 0,
            }
        ));
        assert_eq!(service.resolve(saturated.token()).expect("payload intact"), b"x".to_vec());
        assert_eq!(
            service.stats().expect("saturated stats"),
            PreparedPublicationStatsV1 {
                prepared_count: 1,
                charged_bytes: 1,
                tombstone_count: MAX_TOMBSTONES,
                tombstone_bytes: MAX_TOMBSTONES as u64 * TOMBSTONE_BYTES_PER_CYCLE,
            }
        );
    }

    #[cfg(feature = "core-live-accounting")]
    mod accounting {
        use super::*;
        use quanta_memory_platform_core::{
            ExecutionBudget, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClassBudget,
            MemoryPreset, ReclaimPriority, ResidencyPolicy,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{OnceLock, Weak};

        #[derive(Clone, Debug)]
        struct PanicOnDropPayload {
            bytes: Vec<u8>,
            drops: Arc<AtomicUsize>,
        }

        impl ByteSized for PanicOnDropPayload {
            fn byte_size(&self) -> u64 {
                self.bytes.len() as u64
            }
        }

        impl CanonicalKey for PanicOnDropPayload {
            fn canonical_key(&self) -> Vec<u8> {
                self.bytes.clone()
            }
        }

        impl Drop for PanicOnDropPayload {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::SeqCst);
                panic!("client payload destructor panic");
            }
        }

        struct ReentrantClonePayload {
            bytes: Vec<u8>,
            service: Arc<OnceLock<Weak<PreparedPublicationService<Self, Budgeted>>>>,
            clone_hits: Arc<AtomicUsize>,
        }

        impl Clone for ReentrantClonePayload {
            fn clone(&self) -> Self {
                if let Some(service) = self.service.get().and_then(Weak::upgrade) {
                    assert!(matches!(
                        service.stats(),
                        Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(resource)))
                            if resource == "prepared publication service"
                    ));
                    self.clone_hits.fetch_add(1, Ordering::SeqCst);
                }
                Self {
                    bytes: self.bytes.clone(),
                    service: Arc::clone(&self.service),
                    clone_hits: Arc::clone(&self.clone_hits),
                }
            }
        }

        impl ByteSized for ReentrantClonePayload {
            fn byte_size(&self) -> u64 {
                self.bytes.len() as u64
            }
        }

        impl CanonicalKey for ReentrantClonePayload {
            fn canonical_key(&self) -> Vec<u8> {
                self.bytes.clone()
            }
        }

        fn panic_payload_request(
            identity: &str,
            bytes: &[u8],
            drops: &Arc<AtomicUsize>,
        ) -> PreparedPublicationRequestV1<PanicOnDropPayload> {
            PreparedPublicationRequestV1::new(
                PreparedPublicationIdentityV1::new(identity.as_bytes().to_vec()).expect("identity"),
                PanicOnDropPayload { bytes: bytes.to_vec(), drops: Arc::clone(drops) },
            )
        }

        struct RetireDuringUnwind<'a> {
            service: &'a PreparedPublicationService<PanicOnDropPayload, Budgeted>,
            gc: bool,
        }

        impl Drop for RetireDuringUnwind<'_> {
            fn drop(&mut self) {
                if self.gc {
                    self.service.gc_orphans(&[]).expect("orphan GC during unwind");
                } else {
                    self.service.close().expect("close during unwind");
                }
            }
        }

        fn broker(class_hard: u64) -> LiveProcessMemoryBroker {
            let residency = ResidencyPolicy::new(1_000_000).with_class_budget(
                MemoryClass::ManifestResidency,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: class_hard,
                    hard_cap_bytes: class_hard,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            );
            quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
                .with_manifest(MemoryBudgetManifest::new(
                    MemoryPreset::Production,
                    ExecutionBudget::default(),
                    residency,
                ))
                .build()
                .expect("valid broker")
        }

        fn charged(
            broker: &LiveProcessMemoryBroker,
        ) -> PreparedPublicationService<Vec<u8>, Budgeted> {
            PreparedPublicationService::new_budgeted(binding(), None, None, broker.clone())
        }

        #[test]
        fn resolve_clone_reentry_under_read_lock_is_typed_busy_and_preserves_charge() {
            let broker = broker(1_000_000);
            let service = Arc::new(
                PreparedPublicationService::<ReentrantClonePayload, Budgeted>::new_budgeted(
                    binding(),
                    None,
                    None,
                    broker.clone(),
                ),
            );
            let probe = Arc::new(OnceLock::new());
            assert!(probe.set(Arc::downgrade(&service)).is_ok());
            let clone_hits = Arc::new(AtomicUsize::new(0));
            let receipt = service
                .prepare(PreparedPublicationRequestV1::new(
                    PreparedPublicationIdentityV1::new(b"clone-reentry".to_vec())
                        .expect("identity"),
                    ReentrantClonePayload {
                        bytes: b"owned".to_vec(),
                        service: Arc::clone(&probe),
                        clone_hits: Arc::clone(&clone_hits),
                    },
                ))
                .expect("prepare");

            let resolved = service.resolve(receipt.token()).expect("resolve without deadlock");
            assert_eq!(resolved.bytes, b"owned");
            assert_eq!(clone_hits.load(Ordering::SeqCst), 1);
            assert_eq!(service.stats().expect("complete state").charged_bytes, 5);
            assert_eq!(broker.ledger_snapshot().expect("unchanged charge").total_owner_bytes(), 5);
            service.close().expect("close");
            assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
        }

        #[test]
        fn replay_charges_once_and_abort_refunds_exactly_once() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            let first = service.prepare(request("id", b"payload")).expect("first");
            let replay = service.prepare(request("id", b"payload")).expect("replay");
            assert_eq!(first.token(), replay.token());
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 7);
            assert_eq!(service.abort(first.token()).expect("abort"), AbortDispositionV1::Aborted);
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
            assert_eq!(service.abort(first.token()).expect("replay"), AbortDispositionV1::Replayed);
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
        }

        #[test]
        fn failed_abort_keeps_payload_index_and_charge_coherent() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            let receipt = service.prepare(request("id", b"payload")).expect("prepare");
            broker.begin_rebuild().expect("begin rebuild");

            let denied = service.abort(receipt.token());
            assert!(matches!(denied, Err(PreparedPublicationErrorV1::Accounting(_))));
            assert_eq!(service.resolve(receipt.token()).expect("still prepared"), b"payload");
            assert_eq!(service.stats().expect("stats").charged_bytes, 7);
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 7);

            broker.finish_rebuild().expect("finish rebuild");
            assert_eq!(service.abort(receipt.token()).expect("abort"), AbortDispositionV1::Aborted);
        }

        #[test]
        fn broker_capacity_denial_leaves_no_local_or_ledger_state() {
            let broker = broker(4);
            let service = charged(&broker);
            let denied = service.prepare(request("id", b"payload"));
            assert!(matches!(
                denied,
                Err(PreparedPublicationErrorV1::Accounting(MemoryError::Capacity(_)))
            ));
            assert_eq!(service.stats().expect("stats"), PreparedPublicationStatsV1::default());
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
        }

        #[test]
        fn failed_orphan_gc_preserves_every_payload_index_and_charge() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            let first = service.prepare(request("id-1", b"one")).expect("first");
            let second = service.prepare(request("id-2", b"two")).expect("second");
            broker.begin_rebuild().expect("begin rebuild");

            let denied = service.gc_orphans(&[]);
            assert!(matches!(denied, Err(PreparedPublicationErrorV1::Accounting(_))));
            assert_eq!(service.resolve(first.token()).expect("first intact"), b"one".to_vec());
            assert_eq!(service.resolve(second.token()).expect("second intact"), b"two".to_vec());
            assert_eq!(
                service.stats().expect("stats"),
                PreparedPublicationStatsV1 {
                    prepared_count: 2,
                    charged_bytes: 6,
                    ..PreparedPublicationStatsV1::default()
                }
            );
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 6);

            broker.finish_rebuild().expect("finish rebuild");
            assert_eq!(service.gc_orphans(&[]).expect("retry GC"), 2);
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
        }

        #[test]
        fn close_refunds_all_entries_and_is_terminal_and_idempotent() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            let first = service.prepare(request("id-1", b"one")).expect("first");
            service.prepare(request("id-2", b"two")).expect("second");
            assert_eq!(broker.ledger_snapshot().expect("before").total_owner_bytes(), 6);

            service.close().expect("close");
            assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
            assert_eq!(
                service.stats().expect("terminal stats"),
                PreparedPublicationStatsV1::default()
            );
            assert!(matches!(
                service.resolve(first.token()),
                Err(PreparedPublicationErrorV1::UnknownToken { .. })
            ));
            assert!(matches!(
                service.prepare(request("id-3", b"three")),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::Closed(_)))
            ));
            assert!(matches!(
                service.abort(first.token()),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::Closed(_)))
            ));
            assert!(matches!(
                service.gc_orphans(&[]),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::Closed(_)))
            ));
            service.close().expect("double close");
            assert_eq!(
                broker.ledger_snapshot().expect("double close ledger").total_owner_bytes(),
                0
            );
        }

        #[test]
        fn failed_close_keeps_exact_payloads_tokens_and_charges_for_retry() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            let first = service.prepare(request("id-1", b"one")).expect("first");
            let second = service.prepare(request("id-2", b"two")).expect("second");
            let before = service.stats().expect("before");
            broker.begin_rebuild().expect("begin rebuild");

            assert!(matches!(service.close(), Err(PreparedPublicationErrorV1::Accounting(_))));
            assert_eq!(service.stats().expect("after failed close"), before);
            assert_eq!(service.resolve(first.token()).expect("first intact"), b"one");
            assert_eq!(service.resolve(second.token()).expect("second intact"), b"two");
            assert_eq!(broker.ledger_snapshot().expect("still charged").total_owner_bytes(), 6);

            broker.finish_rebuild().expect("finish rebuild");
            service.close().expect("retry close");
            assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
        }

        #[test]
        fn drop_refunds_live_entries_and_rebuild_drop_defers_exact_release() {
            let broker = broker(1_000_000);
            {
                let service = charged(&broker);
                service.prepare(request("id-1", b"one")).expect("first");
                service.prepare(request("id-2", b"two")).expect("second");
                assert_eq!(broker.ledger_snapshot().expect("live").total_owner_bytes(), 6);
            }
            assert_eq!(broker.ledger_snapshot().expect("healthy drop").total_owner_bytes(), 0);

            {
                let service = charged(&broker);
                service.prepare(request("id-3", b"three")).expect("third");
                broker.begin_rebuild().expect("begin rebuild");
                assert_eq!(broker.ledger_snapshot().expect("before drop").total_owner_bytes(), 5);
            }
            assert_eq!(broker.ledger_snapshot().expect("deferred").total_owner_bytes(), 5);
            broker.finish_rebuild().expect("drain deferred");
            assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        }

        #[test]
        fn multiple_panicking_client_drops_during_outer_unwind_drain_exact_charges() {
            for operation in ["drop", "close", "gc"] {
                let broker = broker(1_000_000);
                let drops = Arc::new(AtomicUsize::new(0));
                let service =
                    PreparedPublicationService::<PanicOnDropPayload, Budgeted>::new_budgeted(
                        binding(),
                        None,
                        None,
                        broker.clone(),
                    );
                for (identity, bytes) in
                    [("one", b"a".as_slice()), ("two", b"bb"), ("three", b"ccc")]
                {
                    service
                        .prepare(panic_payload_request(identity, bytes, &drops))
                        .expect("prepare panicking payload");
                }
                assert_eq!(broker.ledger_snapshot().expect("before unwind").total_owner_bytes(), 6);

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match operation {
                        "drop" => {
                            // The owned service itself must be destroyed while
                            // the outer panic is already unwinding.
                            let _owned = service;
                            panic!("outer unwind");
                        }
                        "close" | "gc" => {
                            let _retire =
                                RetireDuringUnwind { service: &service, gc: operation == "gc" };
                            panic!("outer unwind");
                        }
                        _ => unreachable!(),
                    }
                }));
                assert!(result.is_err(), "the outer panic must remain observable");
                assert_eq!(drops.load(Ordering::SeqCst), 3, "{operation} must retire all payloads");
                assert_eq!(broker.ledger_snapshot().expect("after unwind").total_owner_bytes(), 0);
            }
        }

        #[test]
        fn mutation_gate_returns_typed_busy_and_exposes_no_partial_plan() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            let receipt = service.prepare(request("id", b"payload")).expect("prepare");
            let guard = service.lifecycle.begin_mutation().expect("own lane");
            assert!(matches!(
                service.prepare(request("other", b"x")),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
            ));
            assert!(matches!(
                service.abort(receipt.token()),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
            ));
            assert!(matches!(
                service.gc_orphans(&[]),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
            ));
            assert!(matches!(
                service.close(),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
            ));
            assert!(matches!(
                service.stats(),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
            ));
            drop(guard);
            assert_eq!(service.stats().expect("reopened").charged_bytes, 7);
        }

        #[test]
        fn poisoned_store_reports_error_but_drop_still_releases_every_charge() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            service.prepare(request("id", b"payload")).expect("prepare");
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = service.state.lock().expect("healthy lock");
                panic!("poison prepared publication state");
            }));
            assert!(matches!(service.stats(), Err(PreparedPublicationErrorV1::Storage(_))));
            assert!(matches!(service.close(), Err(PreparedPublicationErrorV1::Storage(_))));
            assert_eq!(broker.ledger_snapshot().expect("still charged").total_owner_bytes(), 7);
            drop(service);
            assert_eq!(broker.ledger_snapshot().expect("drop refunds").total_owner_bytes(), 0);
        }

        #[test]
        fn checked_overflow_and_underflow_reject_before_broker_or_index_mutation() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            {
                let mut state = service.state.lock().expect("state");
                state.charged_bytes = u64::MAX;
            }
            assert!(matches!(
                service.prepare(request("overflow", b"x")),
                Err(PreparedPublicationErrorV1::Capacity { .. })
            ));
            assert_eq!(broker.ledger_snapshot().expect("no charge").total_owner_bytes(), 0);
            {
                let mut state = service.state.lock().expect("state");
                state.charged_bytes = 0;
            }
            let receipt = service.prepare(request("id", b"payload")).expect("prepare");
            {
                let mut state = service.state.lock().expect("state");
                state.charged_bytes = 0;
            }
            assert!(matches!(
                service.abort(receipt.token()),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(_)))
            ));
            assert_eq!(service.resolve(receipt.token()).expect("index intact"), b"payload");
            assert_eq!(broker.ledger_snapshot().expect("charge intact").total_owner_bytes(), 7);
            {
                let mut state = service.state.lock().expect("state");
                state.charged_bytes = 7;
            }
            assert_eq!(service.abort(receipt.token()).expect("retry"), AbortDispositionV1::Aborted);
            assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
        }

        #[test]
        fn orphan_gc_batch_sum_overflow_cannot_partially_refund_or_tombstone() {
            let broker = broker(1_000_000);
            let service = charged(&broker);
            let first = service.prepare(request("id-1", b"a")).expect("first");
            let second = service.prepare(request("id-2", b"b")).expect("second");
            {
                let mut state = service.state.lock().expect("state");
                for entry in state.entries.values_mut() {
                    entry.charged_bytes = u64::MAX;
                }
            }

            assert!(matches!(
                service.gc_orphans(&[]),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(_)))
            ));
            assert_eq!(service.resolve(first.token()).expect("first intact"), b"a");
            assert_eq!(service.resolve(second.token()).expect("second intact"), b"b");
            assert_eq!(service.stats().expect("stats").tombstone_count, 0);
            assert_eq!(broker.ledger_snapshot().expect("charges intact").total_owner_bytes(), 2);

            {
                let mut state = service.state.lock().expect("state");
                for entry in state.entries.values_mut() {
                    entry.charged_bytes = 1;
                }
            }
            assert_eq!(service.gc_orphans(&[]).expect("retry"), 2);
            assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
        }
    }
}
