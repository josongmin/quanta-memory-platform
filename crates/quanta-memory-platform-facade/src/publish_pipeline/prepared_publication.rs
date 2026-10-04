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
use crate::shared::lifecycle::{
    retire_client_value, BeginClose, DropTransition, InFlight, StoreLifecycle,
};
use crate::shared::measure::{semantic_cost_v1, ByteSized, CanonicalKey, RetainedCostV1};

const TOKEN_DOMAIN: &[u8] = b"qmp.prepared-publication.v1\0";
const DEFAULT_IDENTITY_EVIDENCE_COUNT_CAPACITY: usize = 4_096;
const DEFAULT_IDENTITY_EVIDENCE_BYTES_CAPACITY: u64 = 4 * 1024 * 1024;
/// Upper bound on the raw caller-supplied reference-root slice accepted by
/// orphan enumeration and GC. Duplicates count toward the raw length. This is
/// a conservative management bound aligned with the broker's live-ticket cap,
/// not a latency or RSS guarantee; normalization runs before any state lock
/// or mutation permit is acquired.
const MAX_REFERENCED_ROOT_TOKENS: usize = 16_384;
// The persistent lease covers the identity-index key and state token, plus
// the token key/index that moves from the prepared map to the aborted set.
// The active lease covers only fields destroyed by abort/GC.
const TOKEN_BYTES: u64 = semantic_cost_v1::DIGEST_BYTES;
const PERSISTENT_INDEX_BYTES: u64 = 2 * semantic_cost_v1::ORDERED_INDEX_BYTES;
const PERSISTENT_FIXED_BYTES: u64 = 2 * TOKEN_BYTES + semantic_cost_v1::ENTRY_STATE_BYTES;
const ACTIVE_FIXED_BYTES: u64 = semantic_cost_v1::ENTRY_STATE_BYTES;

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
    /// Both live prepared identities and aborted identity evidence.
    pub identity_evidence_count: usize,
    pub identity_evidence_bytes: u64,
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
    IdentityEvidenceCapacity {
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
            Self::IdentityEvidenceCapacity {
                requested_count,
                available_count,
                requested_bytes,
                available_bytes,
            } => write!(
                f,
                "prepared publication identity evidence capacity exceeded: requested {requested_count} entries/{requested_bytes} bytes, available {available_count} entries/{available_bytes} bytes"
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
    active_bytes: u64,
    /// Only payload, canonical key, and the entry's second identity copy.
    active_charge: Option<ManagedCharge>,
}

fn retire_entry<Payload>(entry: PreparedEntry<Payload>) {
    let PreparedEntry { payload, active_charge, .. } = entry;
    drop(active_charge);
    retire_client_value(payload);
}

fn retire_entries<Payload>(entries: BTreeMap<PreparedPublicationTokenV1, PreparedEntry<Payload>>) {
    for (_, entry) in entries {
        retire_entry(entry);
    }
}

#[derive(Debug)]
enum IdentityState {
    Prepared(PreparedPublicationTokenV1),
    Aborted(PreparedPublicationTokenV1),
}

#[derive(Debug)]
struct IdentityEvidence {
    state: IdentityState,
    retained_bytes: u64,
    /// Survives abort/GC along with the identity index and token tombstone.
    charge: Option<ManagedCharge>,
}

#[derive(Debug)]
struct PreparedState<Payload> {
    entries: BTreeMap<PreparedPublicationTokenV1, PreparedEntry<Payload>>,
    identity_index: BTreeMap<PreparedPublicationIdentityV1, IdentityEvidence>,
    aborted_tokens: BTreeSet<PreparedPublicationTokenV1>,
    charged_bytes: u64,
    tombstone_bytes: u64,
    /// Persistent identity evidence reserved for both live and aborted entries.
    /// This is a capacity reservation, not a second accounting charge.
    persistent_bytes: u64,
}

impl<Payload> PreparedState<Payload> {
    fn empty() -> Self {
        Self {
            entries: BTreeMap::new(),
            identity_index: BTreeMap::new(),
            aborted_tokens: BTreeSet::new(),
            charged_bytes: 0,
            tombstone_bytes: 0,
            persistent_bytes: 0,
        }
    }
}

fn measured_len(len: usize) -> Result<u64, PreparedPublicationErrorV1> {
    u64::try_from(len).map_err(|_| {
        PreparedPublicationErrorV1::Accounting(MemoryError::capacity(
            "prepared publication retained length does not fit u64",
        ))
    })
}

fn persistent_cost(identity_len: usize) -> Result<u64, PreparedPublicationErrorV1> {
    RetainedCostV1 {
        key_identity_bytes: measured_len(identity_len)?,
        index_bytes: PERSISTENT_INDEX_BYTES,
        fixed_overhead_bytes: PERSISTENT_FIXED_BYTES,
        ..RetainedCostV1::default()
    }
    .total()
    .map_err(PreparedPublicationErrorV1::Accounting)
}

fn active_cost(
    payload_bytes: u64,
    identity_len: usize,
    canonical_key_len: usize,
) -> Result<u64, PreparedPublicationErrorV1> {
    let identity_bytes = measured_len(identity_len)?;
    let key_bytes = measured_len(canonical_key_len)?;
    let key_identity_bytes = identity_bytes.checked_add(key_bytes).ok_or_else(|| {
        PreparedPublicationErrorV1::Accounting(MemoryError::capacity(
            "prepared publication active identity/key cost overflow",
        ))
    })?;
    RetainedCostV1 {
        payload_bytes,
        key_identity_bytes,
        fixed_overhead_bytes: ACTIVE_FIXED_BYTES,
        ..RetainedCostV1::default()
    }
    .total()
    .map_err(PreparedPublicationErrorV1::Accounting)
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
/// let _ = PreparedPublicationService::<Vec<u8>, Budgeted>::charged_against;
/// ```
#[derive(Debug)]
pub struct PreparedPublicationService<Payload, Mode> {
    binding: StoreBindingHandle,
    capacity_bytes: Option<u64>,
    identity_evidence_count_capacity: usize,
    identity_evidence_bytes_capacity: u64,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    state: Mutex<PreparedState<Payload>>,
    _mode: PhantomData<fn() -> Mode>,
}

impl<Payload> PreparedPublicationService<Payload, Budgeted> {
    pub(crate) fn new_budgeted(
        binding: StoreBindingHandle,
        capacity_bytes: Option<u64>,
        identity_evidence_limits: Option<(usize, u64)>,
        broker: LiveProcessMemoryBroker,
    ) -> Self {
        Self::with_accounting(
            binding,
            capacity_bytes,
            identity_evidence_limits,
            StoreAccounting::Budgeted(broker),
        )
    }
}

#[cfg(feature = "unbudgeted-stores")]
impl<Payload> PreparedPublicationService<Payload, Unbudgeted> {
    pub(crate) fn new_unbudgeted(
        binding: StoreBindingHandle,
        capacity_bytes: Option<u64>,
        identity_evidence_limits: Option<(usize, u64)>,
    ) -> Self {
        Self::with_accounting(
            binding,
            capacity_bytes,
            identity_evidence_limits,
            StoreAccounting::Unbudgeted,
        )
    }
}

impl<Payload, Mode> PreparedPublicationService<Payload, Mode> {
    fn with_accounting(
        binding: StoreBindingHandle,
        capacity_bytes: Option<u64>,
        identity_evidence_limits: Option<(usize, u64)>,
        accounting: StoreAccounting,
    ) -> Self {
        let (identity_evidence_count_capacity, identity_evidence_bytes_capacity) =
            identity_evidence_limits.unwrap_or((
                DEFAULT_IDENTITY_EVIDENCE_COUNT_CAPACITY,
                DEFAULT_IDENTITY_EVIDENCE_BYTES_CAPACITY,
            ));
        Self {
            binding,
            capacity_bytes,
            identity_evidence_count_capacity,
            identity_evidence_bytes_capacity,
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

    /// One public read through the store-owned boundary: lifecycle preflight,
    /// nonblocking state lock, and unwind containment for caller code on the
    /// locked state. Readers never observe entries while their accounting
    /// authority is in an out-of-lock release plan.
    fn with_read<R>(
        &self,
        read: impl FnOnce(&PreparedState<Payload>) -> R,
    ) -> Result<R, PreparedPublicationErrorV1> {
        self.lifecycle
            .with_read_state(&self.state, read)
            .map_err(PreparedPublicationErrorV1::Storage)
    }

    fn require_charge_shape(
        &self,
        charge: Option<&ManagedCharge>,
        expected_kind: ChargeKind,
        bytes: u64,
        operation: &str,
    ) -> Result<(), PreparedPublicationErrorV1> {
        if self.accounting.broker().is_some() == charge.is_some()
            && charge.is_none_or(|charge| {
                charge.bytes() == Some(bytes) && charge.kind() == Some(expected_kind)
            })
        {
            Ok(())
        } else {
            Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(format!(
                "prepared publication {operation} encountered mismatched accounting authority"
            ))))
        }
    }

    /// Release active payload and persistent identity/tombstone leases atomically.
    /// A failed release restores the exact preimage and reopens the service.
    pub fn close(&self) -> Result<(), PreparedPublicationErrorV1> {
        let close =
            match self.lifecycle.begin_close().map_err(PreparedPublicationErrorV1::Storage)? {
                BeginClose::AlreadyClosed => return Ok(()),
                BeginClose::Acquired(close) => close,
            };

        enum LeaseOwner {
            Active(PreparedPublicationTokenV1),
            Evidence(PreparedPublicationIdentityV1),
        }

        let (owners, mut charges) = {
            let mut state = match self.lock_state() {
                Ok(state) => state,
                Err(error) => {
                    close.rollback();
                    return Err(error);
                }
            };
            for entry in state.entries.values() {
                if let Err(error) = self.require_charge_shape(
                    entry.active_charge.as_ref(),
                    ChargeKind::RetainedPayload,
                    entry.active_bytes,
                    "close active",
                ) {
                    close.rollback();
                    return Err(error);
                }
            }
            for evidence in state.identity_index.values() {
                if let Err(error) = self.require_charge_shape(
                    evidence.charge.as_ref(),
                    ChargeKind::RetainedMetadata,
                    evidence.retained_bytes,
                    "close identity evidence",
                ) {
                    close.rollback();
                    return Err(error);
                }
            }
            let mut owners = Vec::new();
            let mut charges = Vec::new();
            for (token, entry) in &mut state.entries {
                if let Some(charge) = entry.active_charge.take() {
                    owners.push(LeaseOwner::Active(token.clone()));
                    charges.push(charge);
                }
            }
            for (identity, evidence) in &mut state.identity_index {
                if let Some(charge) = evidence.charge.take() {
                    owners.push(LeaseOwner::Evidence(identity.clone()));
                    charges.push(charge);
                }
            }
            (owners, charges)
        };

        if let Err(error) = self.accounting.release_many(&mut charges) {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            for (owner, charge) in owners.into_iter().zip(charges) {
                match owner {
                    LeaseOwner::Active(token) => {
                        state
                            .entries
                            .get_mut(&token)
                            .expect("closing lane owns entry")
                            .active_charge = Some(charge);
                    }
                    LeaseOwner::Evidence(identity) => {
                        state
                            .identity_index
                            .get_mut(&identity)
                            .expect("closing lane owns evidence")
                            .charge = Some(charge);
                    }
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
        drop(old_state.identity_index);
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
        let evidence = std::mem::take(&mut state.identity_index);
        state.charged_bytes = 0;
        state.tombstone_bytes = 0;
        state.persistent_bytes = 0;
        drop(evidence);
        retire_entries(entries);
    }
}

impl<Payload, Mode> PreparedPublicationService<Payload, Mode>
where
    Payload: Clone + ByteSized + CanonicalKey,
{
    pub fn resolve(
        &self,
        token: &PreparedPublicationTokenV1,
    ) -> Result<Payload, PreparedPublicationErrorV1> {
        self.with_read(|state| {
            if let Some(entry) = state.entries.get(token) {
                return Ok(entry.payload.clone());
            }
            if state.aborted_tokens.contains(token) {
                return Err(PreparedPublicationErrorV1::AbortedToken { token: token.clone() });
            }
            Err(PreparedPublicationErrorV1::UnknownToken { token: token.clone() })
        })?
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
            self.require_charge_shape(
                entry.active_charge.as_ref(),
                ChargeKind::RetainedPayload,
                entry.active_bytes,
                "abort active",
            )?;
            let evidence = state.identity_index.get(&entry.identity).ok_or_else(|| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "prepared entry has no identity evidence".to_string(),
                ))
            })?;
            if !matches!(&evidence.state, IdentityState::Prepared(indexed) if indexed == token) {
                return Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "prepared identity evidence does not point to entry".to_string(),
                )));
            }
            self.require_charge_shape(
                evidence.charge.as_ref(),
                ChargeKind::RetainedMetadata,
                evidence.retained_bytes,
                "abort identity evidence",
            )?;
            let tombstone_bytes = self.reserved_retirement_bytes(&state, [entry])?;
            let next_total =
                state.charged_bytes.checked_sub(entry.active_bytes).ok_or_else(|| {
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
            let charge = state.entries.get_mut(token).and_then(|entry| entry.active_charge.take());
            (next_total, next_tombstone_bytes, charge)
        };
        if let Some(active) = charge.as_mut() {
            if let Err(error) = self.accounting.release_many(std::slice::from_mut(active)) {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.entries.get_mut(token).expect("mutation lane owns entry").active_charge =
                    charge;
                return Err(PreparedPublicationErrorV1::Accounting(error));
            }
        }
        let entry = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let (owned_token, entry) =
                state.entries.remove_entry(token).expect("mutation lane owns entry");
            state.charged_bytes = next_total;
            state
                .identity_index
                .get_mut(&entry.identity)
                .expect("mutation lane owns evidence")
                .state = IdentityState::Aborted(token.clone());
            state.aborted_tokens.insert(owned_token);
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
    /// reference fails closed. The raw slice is bounded and normalized before
    /// the read boundary is acquired.
    pub fn orphan_tokens(
        &self,
        referenced_tokens: &[PreparedPublicationTokenV1],
    ) -> Result<Vec<PreparedPublicationTokenV1>, PreparedPublicationErrorV1> {
        let referenced = normalize_referenced_tokens(referenced_tokens)?;
        self.with_read(|state| classify_orphan_tokens(state, &referenced))?
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
        let referenced = normalize_referenced_tokens(referenced_tokens)?;
        let _permit =
            self.lifecycle.begin_mutation().map_err(PreparedPublicationErrorV1::Storage)?;
        let (prepared_tokens, charged_tokens, next_total, next_tombstone_bytes, mut charges) = {
            let mut state = self.lock_state()?;
            let prepared_tokens = classify_orphan_tokens(&state, &referenced)?;
            let tombstone_entries = prepared_tokens
                .iter()
                .map(|token| state.entries.get(token).expect("classified token is live"))
                .collect::<Vec<_>>();
            for (token, entry) in prepared_tokens.iter().zip(&tombstone_entries) {
                self.require_charge_shape(
                    entry.active_charge.as_ref(),
                    ChargeKind::RetainedPayload,
                    entry.active_bytes,
                    "orphan GC active",
                )?;
                let evidence = state.identity_index.get(&entry.identity).ok_or_else(|| {
                    PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                        "orphan GC entry has no identity evidence".to_string(),
                    ))
                })?;
                if !matches!(&evidence.state, IdentityState::Prepared(indexed) if indexed == token)
                {
                    return Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                        "orphan GC identity evidence does not point to entry".to_string(),
                    )));
                }
                self.require_charge_shape(
                    evidence.charge.as_ref(),
                    ChargeKind::RetainedMetadata,
                    evidence.retained_bytes,
                    "orphan GC identity evidence",
                )?;
            }
            let tombstone_bytes =
                self.reserved_retirement_bytes(&state, tombstone_entries.iter().copied())?;
            let reclaimed_bytes = tombstone_entries.iter().try_fold(0u64, |total, entry| {
                total.checked_add(entry.active_bytes).ok_or_else(|| {
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
                if let Some(charge) = state
                    .entries
                    .get_mut(token)
                    .expect("classified token is live")
                    .active_charge
                    .take()
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
                state.entries.get_mut(&token).expect("mutation lane owns entry").active_charge =
                    Some(charge);
            }
            return Err(PreparedPublicationErrorV1::Accounting(error));
        }
        let removed = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let mut removed = Vec::with_capacity(prepared_tokens.len());
            for token in &prepared_tokens {
                let (owned_token, entry) =
                    state.entries.remove_entry(token).expect("mutation lane owns entry");
                state
                    .identity_index
                    .get_mut(&entry.identity)
                    .expect("mutation lane owns evidence")
                    .state = IdentityState::Aborted(token.clone());
                state.aborted_tokens.insert(owned_token);
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
        self.with_read(|state| PreparedPublicationStatsV1 {
            prepared_count: state.entries.len(),
            charged_bytes: state.charged_bytes,
            identity_evidence_count: state.identity_index.len(),
            identity_evidence_bytes: state.persistent_bytes,
            tombstone_count: state.aborted_tokens.len(),
            tombstone_bytes: state.tombstone_bytes,
        })
    }

    /// Retirement consumes space reserved by fresh prepare. Check local
    /// accounting consistency without making another capacity decision.
    fn reserved_retirement_bytes<'a>(
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
            let evidence = state.identity_index.get(&entry.identity).ok_or_else(|| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "prepared entry has no identity evidence".to_string(),
                ))
            })?;
            requested_bytes =
                requested_bytes.checked_add(evidence.retained_bytes).ok_or_else(|| {
                    PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                        "prepared publication tombstone-byte counter overflow".to_string(),
                    ))
                })?;
        }

        let next_aborted_count =
            state.aborted_tokens.len().checked_add(requested_count).ok_or_else(|| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "prepared publication aborted-count overflow".to_string(),
                ))
            })?;
        let next_tombstone_bytes =
            state.tombstone_bytes.checked_add(requested_bytes).ok_or_else(|| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "prepared publication tombstone-byte overflow".to_string(),
                ))
            })?;
        if next_aborted_count > state.identity_index.len()
            || next_tombstone_bytes > state.persistent_bytes
        {
            return Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                "prepared publication retirement exceeds admitted identity evidence".to_string(),
            )));
        }
        Ok(requested_bytes)
    }
}

/// Bound and normalize a raw caller-supplied reference-root slice.
///
/// The raw length (duplicates included) is rejected above
/// [`MAX_REFERENCED_ROOT_TOKENS`] with a typed storage capacity error before
/// any proportional set is built, so an oversized input cannot allocate or
/// hold the state lock. The fixed-size tokens are then cloned and
/// deduplicated into the set the state validation consumes.
mod helpers;
mod prepare;
use helpers::{classify_orphan_tokens, derive_token, normalize_referenced_tokens};

#[cfg(test)]
mod tests;
