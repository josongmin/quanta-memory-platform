use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, PoisonError};

use hashlink::linked_hash_map::RawEntryMut;
use hashlink::LinkedHashMap;

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, ManagedChargeRechargeRecovery,
    MemoryContextKind, MemoryError, StoreBindingHandle, StoreStats, StoreWriteError, SweepHint,
    SweepStats,
};

use crate::retained_map::{
    RetainedMapInsertOrExistingV1, RetainedMapInsertOutcomeV1, RetainedMapStore,
};
#[cfg(feature = "unbudgeted-stores")]
use crate::shared::accounting::Unbudgeted;
use crate::shared::accounting::{Budgeted, StoreAccounting};
use crate::shared::lifecycle::{
    retire_client_value, BeginClose, DropTransition, InFlight, StoreLifecycle,
};
use crate::shared::measure::{semantic_cost_v1, ByteSized, RetainedCostV1};

#[derive(Debug)]
struct MapEntry<Value> {
    value: Value,
    retained_cost: RetainedCostV1,
    charged_bytes: u64,
    /// Single-owner ledger authority in budgeted mode; absent only for the
    /// explicitly distinct unbudgeted product type.
    charge: Option<ManagedCharge>,
    /// Private, mutation-lane-only sweep plan bit. Never exposed to readers.
    sweep_selected: bool,
}

#[derive(Debug)]
struct MapState<Key, Value> {
    /// Single canonical payload/index/order owner. Randomized hashing prevents
    /// attacker-selected keys from deterministically degrading point operations;
    /// the linked order preserves newest-first sweep semantics.
    entries: LinkedHashMap<Key, MapEntry<Value>, RandomState>,
    committed_bytes: u64,
}

fn retire_map_entries<Key, Value>(entries: impl IntoIterator<Item = (Key, MapEntry<Value>)>) {
    for (key, entry) in entries {
        let MapEntry { value, charge, .. } = entry;
        drop(charge);
        retire_client_value(key);
        retire_client_value(value);
    }
}

impl<Key, Value> MapState<Key, Value> {
    fn empty() -> Self {
        Self { entries: LinkedHashMap::with_hasher(RandomState::new()), committed_bytes: 0 }
    }
}

/// Owns the canonical map while a broker operation runs outside its mutex.
/// Unwinding through a user-defined Hash/Clone/Drop callback cannot discard
/// entries or their non-clone accounting leases: the preimage is reinstalled
/// before the lifecycle mutation permit reopens the store.
struct MapStatePlan<'a, Key, Value> {
    slot: &'a Mutex<MapState<Key, Value>>,
    state: Option<MapState<Key, Value>>,
}

impl<Key, Value> Deref for MapStatePlan<'_, Key, Value> {
    type Target = MapState<Key, Value>;

    fn deref(&self) -> &Self::Target {
        self.state.as_ref().expect("map plan owns the canonical state until drop")
    }
}

impl<Key, Value> DerefMut for MapStatePlan<'_, Key, Value> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.state.as_mut().expect("map plan owns the canonical state until drop")
    }
}

impl<Key, Value> Drop for MapStatePlan<'_, Key, Value> {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            *slot = state;
        }
    }
}

/// Portable retained map store.
///
/// Owns the reserve -> mutate -> commit / release choreography (playbook 4.1):
/// envelope bytes are estimated and tentatively reserved, the map is mutated, and
/// only a successful mutation commits the reservation; a failed mutation releases
/// the tentative bytes so accounting never drifts.
///
/// `Mode` is intentionally mandatory: budgeted and unbudgeted stores are
/// different product types. Production construction starts at
/// [`crate::MemoryPlatformBuilder`]; the raw concrete type has no public
/// constructor and cannot acquire accounting authority after materialization.
///
/// ```compile_fail
/// use quanta_memory_platform_core::{LiveProcessMemoryBroker, MemoryClass, StoreBindingHandle};
/// use quanta_memory_platform_facade::{Budgeted, SimpleRetainedMapStore};
///
/// let binding = StoreBindingHandle::new(MemoryClass::MemoResidency, "owner".into(), None);
/// let broker: LiveProcessMemoryBroker = unimplemented!();
/// let _ = SimpleRetainedMapStore::<String, Vec<u8>, Budgeted>::new_budgeted(binding, broker);
/// ```
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{Budgeted, SimpleRetainedMapStore};
///
/// let _ = SimpleRetainedMapStore::<String, Vec<u8>, Budgeted>::charged_against;
/// ```
#[derive(Debug)]
pub struct SimpleRetainedMapStore<Key, Value, Mode> {
    binding: StoreBindingHandle,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    state: Mutex<MapState<Key, Value>>,
    _mode: PhantomData<Mode>,
}

impl<Key, Value> SimpleRetainedMapStore<Key, Value, Budgeted> {
    pub(crate) fn new_budgeted(
        binding: StoreBindingHandle,
        broker: LiveProcessMemoryBroker,
    ) -> Self {
        Self::with_accounting(binding, StoreAccounting::Budgeted(broker))
    }
}

#[cfg(feature = "unbudgeted-stores")]
impl<Key, Value> SimpleRetainedMapStore<Key, Value, Unbudgeted> {
    pub(crate) fn new_unbudgeted(binding: StoreBindingHandle) -> Self {
        Self::with_accounting(binding, StoreAccounting::Unbudgeted)
    }
}

#[cfg(test)]
#[derive(Debug)]
struct TestUnbudgeted;

#[cfg(test)]
impl<Key, Value> SimpleRetainedMapStore<Key, Value, TestUnbudgeted> {
    fn new_unbudgeted_fixture(binding: StoreBindingHandle) -> Self {
        Self::with_accounting(binding, StoreAccounting::Unbudgeted)
    }
}

impl<Key, Value, Mode> SimpleRetainedMapStore<Key, Value, Mode> {
    fn with_accounting(binding: StoreBindingHandle, accounting: StoreAccounting) -> Self {
        Self {
            binding,
            accounting,
            lifecycle: StoreLifecycle::new("retained map"),
            state: Mutex::new(MapState::empty()),
            _mode: PhantomData,
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, MapState<Key, Value>>, MemoryError> {
        self.lifecycle.try_lock_state(&self.state)
    }

    fn take_state(&self) -> Result<MapStatePlan<'_, Key, Value>, MemoryError> {
        let mut state = self.lock()?;
        let owned = std::mem::replace(&mut *state, MapState::empty());
        Ok(MapStatePlan { slot: &self.state, state: Some(owned) })
    }

    /// Preserve charge ownership even if a reader poisoned the mutex while the
    /// canonical state was temporarily owned by a mutation plan.
    fn restore_state(&self, state: MapStatePlan<'_, Key, Value>) {
        drop(state);
    }

    fn require_charge_shape(
        &self,
        entry: &MapEntry<Value>,
        operation: &str,
    ) -> Result<(), MemoryError> {
        if self.accounting.broker().is_some() == entry.charge.is_some() {
            Ok(())
        } else {
            Err(MemoryError::InvalidState(format!(
                "retained-map {operation} encountered an entry with mismatched accounting mode"
            )))
        }
    }

    /// Release all live charges atomically and terminally close the store.
    /// Failure restores the exact entries, linked order, counters, and charge
    /// authorities before reopening the lifecycle gate.
    pub fn close(&self) -> Result<(), MemoryError> {
        let close = match self.lifecycle.begin_close()? {
            BeginClose::AlreadyClosed => return Ok(()),
            BeginClose::Acquired(close) => close,
        };
        let mut state = match self.take_state() {
            Ok(state) => state,
            Err(error) => {
                close.rollback();
                return Err(error);
            }
        };

        if let Some(error) =
            state.entries.values().find_map(|entry| self.require_charge_shape(entry, "close").err())
        {
            self.restore_state(state);
            close.rollback();
            return Err(error);
        }

        let mut charges: Vec<ManagedCharge> =
            state.entries.values_mut().filter_map(|entry| entry.charge.take()).collect();
        if let Err(error) = self.accounting.release_many(&mut charges) {
            let mut charges = charges.into_iter();
            for entry in state.entries.values_mut() {
                if entry.charge.is_none() {
                    entry.charge = charges.next();
                }
            }
            debug_assert!(charges.next().is_none());
            self.restore_state(state);
            close.rollback();
            return Err(error);
        }

        // Commit the terminal image before user-defined Key/Value destructors
        // run. A destructor panic must not reopen a ticketless live entry.
        let retired_entries =
            std::mem::replace(&mut state.entries, LinkedHashMap::with_hasher(RandomState::new()));
        state.committed_bytes = 0;
        self.restore_state(state);
        close.commit();
        retire_map_entries(retired_entries);
        Ok(())
    }
}

impl<Key, Value, Mode> Drop for SimpleRetainedMapStore<Key, Value, Mode> {
    fn drop(&mut self) {
        if self.lifecycle.transition_to_closed_for_drop() == DropTransition::CleanupRequired {
            let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
            let entries = std::mem::replace(
                &mut state.entries,
                LinkedHashMap::with_hasher(RandomState::new()),
            );
            state.committed_bytes = 0;
            retire_map_entries(entries);
        }
    }
}

impl<Key, Value, Mode> SimpleRetainedMapStore<Key, Value, Mode>
where
    Key: Clone + Eq + Hash + ByteSized,
    Value: Clone + ByteSized,
{
    /// Committed retained bytes (sum of live entry charges).
    pub fn committed_bytes(&self) -> Result<u64, MemoryError> {
        self.lifecycle.with_read_state(&self.state, |state| state.committed_bytes)
    }

    fn committed_bytes_after_replace_v1(
        committed_bytes: u64,
        removed_bytes: u64,
        added_bytes: u64,
    ) -> Result<u64, StoreWriteError> {
        let retained_without_old = committed_bytes.checked_sub(removed_bytes).ok_or_else(|| {
            MemoryError::InvalidState(
                "retained-map committed bytes are smaller than the replaced entry".to_string(),
            )
        })?;
        retained_without_old
            .checked_add(added_bytes)
            .ok_or_else(|| MemoryError::capacity("retained-map committed byte accounting overflow"))
    }

    /// Publish `value` only when `key` is absent, otherwise return the existing
    /// value observed under the same canonical state lock.
    ///
    /// The occupied path performs no charge, mutation, or insertion-order
    /// refresh. Callers can therefore validate immutable payload metadata
    /// without a `get` -> `insert` TOCTOU window.
    pub fn insert_if_absent_or_get_checked(
        &self,
        key: Key,
        value: Value,
    ) -> Result<RetainedMapInsertOrExistingV1<Value>, StoreWriteError> {
        self.insert_if_absent_with_v1(
            key,
            value,
            |existing| RetainedMapInsertOrExistingV1::AlreadyPresent(existing.clone()),
            RetainedMapInsertOrExistingV1::Inserted,
        )
    }

    fn insert_if_absent_with_v1<Outcome>(
        &self,
        key: Key,
        value: Value,
        on_occupied: impl FnOnce(&Value) -> Outcome,
        inserted: Outcome,
    ) -> Result<Outcome, StoreWriteError> {
        let _permit = self.lifecycle.begin_mutation()?;
        // Measurement, the occupied probe, and the caller's occupied callback
        // all run while the owned pair is guarded: a trait panic unwinds
        // through contained retirement instead of a caller destructor.
        let mut key_in_flight = InFlight::new(key);
        let mut value_in_flight = InFlight::new(value);
        let value_bytes = value_in_flight.get().byte_size();
        let key_bytes = key_in_flight.get().byte_size();
        let mut state = self.take_state()?;
        if let Some(entry) = state.entries.get(key_in_flight.get()) {
            let outcome = on_occupied(&entry.value);
            self.restore_state(state);
            return Ok(outcome);
        }

        let retained_cost = Self::retained_cost_v1(key_bytes, value_bytes);
        let reserved_bytes = match retained_cost.total() {
            Ok(total) => total,
            Err(error) => {
                self.restore_state(state);
                return Err(error);
            }
        };
        let next_committed_bytes = match Self::committed_bytes_after_replace_v1(
            state.committed_bytes,
            0,
            reserved_bytes,
        ) {
            Ok(next) => next,
            Err(error) => {
                self.restore_state(state);
                return Err(error);
            }
        };
        let charge = match self.charge_new_entry_v1(reserved_bytes) {
            Ok(charge) => charge,
            Err(error) => {
                self.restore_state(state);
                return Err(error);
            }
        };
        // Borrowed-key probe with a precomputed hash; the insertion runs no
        // caller Eq and needs no owned caller value. A capacity-growth
        // rehash may still invoke caller Hash on STORED keys inside the
        // insertion — containment there comes from the in-flight guard and
        // the plan's Drop restore, not from trait-freedom.
        let hash = state.entries.hasher().hash_one(key_in_flight.get());
        let RawEntryMut::Vacant(vacant) =
            state.entries.raw_entry_mut().from_key_hashed_nocheck(hash, key_in_flight.get())
        else {
            return Err(MemoryError::InvalidState(
                "retained-map entry appeared between probe and insertion".to_string(),
            ));
        };
        vacant.insert_hashed_nocheck(
            hash,
            key_in_flight.take(),
            MapEntry {
                value: value_in_flight.take(),
                retained_cost,
                charged_bytes: reserved_bytes,
                charge,
                sweep_selected: false,
            },
        );
        state.committed_bytes = next_committed_bytes;
        self.restore_state(state);
        Ok(inserted)
    }

    fn charge_new_entry_v1(
        &self,
        reserved_bytes: u64,
    ) -> Result<Option<ManagedCharge>, StoreWriteError> {
        self.accounting.charge(
            self.binding.class.clone(),
            self.binding.owner.clone(),
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            reserved_bytes,
        )
    }

    fn retained_cost_v1(key_bytes: u64, value_bytes: u64) -> RetainedCostV1 {
        RetainedCostV1 {
            payload_bytes: value_bytes,
            key_identity_bytes: key_bytes,
            index_bytes: semantic_cost_v1::HASH_LINK_BYTES,
            fixed_overhead_bytes: semantic_cost_v1::ENTRY_STATE_BYTES,
            ..RetainedCostV1::default()
        }
    }
}

impl<Key, Value, Mode> RetainedMapStore<Key, Value> for SimpleRetainedMapStore<Key, Value, Mode>
where
    Key: Clone + Eq + Hash + ByteSized,
    Value: Clone + ByteSized,
{
    fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError> {
        let _permit = self.lifecycle.begin_mutation()?;
        // Caller measurement and probes run while the owned pair is guarded:
        // a trait panic unwinds through the guard's contained retirement, not
        // through a caller destructor (compound-hostile containment).
        let mut key_in_flight = InFlight::new(key);
        let mut value_in_flight = InFlight::new(value);
        let value_bytes = value_in_flight.get().byte_size();
        let incoming_key_bytes = key_in_flight.get().byte_size();
        let mut state = self.take_state()?;
        let mut retired_value = None;
        let MapState { entries, committed_bytes } = &mut *state;
        // One keyed entry probe preserves the prior O(1) point-path contract.
        // The probe borrows the in-flight key (caller Hash/Eq never see an
        // owned value in flight); the entry borrow owns only the detached map,
        // never the container mutex, so broker admission/recharge is safe
        // outside the store lock.
        let hash = entries.hasher().hash_one(key_in_flight.get());
        let probe = entries.raw_entry_mut().from_key_hashed_nocheck(hash, key_in_flight.get());
        match probe {
            RawEntryMut::Occupied(mut occupied) => {
                let old_bytes = occupied.get().charged_bytes;
                let retained_cost =
                    RetainedCostV1 { payload_bytes: value_bytes, ..occupied.get().retained_cost };
                let reserved_bytes = retained_cost.total()?;
                let next = Self::committed_bytes_after_replace_v1(
                    *committed_bytes,
                    old_bytes,
                    reserved_bytes,
                )?;
                self.require_charge_shape(occupied.get(), "overwrite")?;
                if let Some(charge) = occupied.get_mut().charge.as_mut() {
                    if let Err(denied) = charge.recharge(
                        self.binding.class.clone(),
                        self.binding.owner.clone(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedMetadata,
                        reserved_bytes,
                    ) {
                        match denied.recovery {
                            ManagedChargeRechargeRecovery::ChargePreserved => {
                                return Err(denied.error);
                            }
                            ManagedChargeRechargeRecovery::RecoveryRequired { rollback_error } => {
                                *committed_bytes =
                                    committed_bytes.checked_sub(old_bytes).ok_or_else(|| {
                                        MemoryError::InvalidState(
                                            "recharge recovery removal exceeded committed bytes"
                                                .to_string(),
                                        )
                                    })?;
                                occupied.remove();
                                return Err(MemoryError::InvalidState(format!(
                                    "recharge rollback failed; uncharged retained payload was removed: primary=[{}], rollback=[{}]",
                                    denied.error, rollback_error
                                )));
                            }
                        }
                    }
                }
                let entry = occupied.get_mut();
                retired_value = Some(std::mem::replace(&mut entry.value, value_in_flight.take()));
                entry.retained_cost = retained_cost;
                entry.charged_bytes = reserved_bytes;
                *committed_bytes = next;
            }
            RawEntryMut::Vacant(vacant) => {
                let retained_cost = Self::retained_cost_v1(incoming_key_bytes, value_bytes);
                let reserved_bytes = retained_cost.total()?;
                let next =
                    Self::committed_bytes_after_replace_v1(*committed_bytes, 0, reserved_bytes)?;
                let charge = self.charge_new_entry_v1(reserved_bytes)?;
                // Insertion with the precomputed hash runs no caller Eq; a
                // growth rehash may still hash STORED keys (contained by the
                // guard and plan restore). The owned key is never in flight
                // during caller Eq.
                vacant.insert_hashed_nocheck(
                    hash,
                    key_in_flight.take(),
                    MapEntry {
                        value: value_in_flight.take(),
                        retained_cost,
                        charged_bytes: reserved_bytes,
                        charge,
                        sweep_selected: false,
                    },
                );
                *committed_bytes = next;
            }
        }
        self.restore_state(state);
        // Value::drop is client code. It must observe a fully published
        // postimage, never an in-flight accounting plan.
        drop(retired_value);
        drop(value_in_flight);
        drop(key_in_flight);
        Ok(())
    }

    fn insert_if_absent_or_get_checked(
        &self,
        key: Key,
        value: Value,
    ) -> Result<RetainedMapInsertOrExistingV1<Value>, StoreWriteError> {
        SimpleRetainedMapStore::insert_if_absent_or_get_checked(self, key, value)
    }

    fn insert_if_absent_checked(
        &self,
        key: Key,
        value: Value,
    ) -> Result<RetainedMapInsertOutcomeV1, StoreWriteError> {
        self.insert_if_absent_with_v1(
            key,
            value,
            |_| RetainedMapInsertOutcomeV1::AlreadyPresent,
            RetainedMapInsertOutcomeV1::Inserted,
        )
    }

    fn get_checked(&self, key: &Key) -> Result<Option<Value>, MemoryError> {
        self.lifecycle.with_read_state(&self.state, |state| {
            state.entries.get(key).map(|entry| entry.value.clone())
        })
    }

    fn remove(&self, key: &Key) -> Result<Option<Value>, MemoryError> {
        let _permit = self.lifecycle.begin_mutation()?;
        let mut state = self.take_state()?;
        let MapState { entries, committed_bytes } = &mut *state;
        // Caller Hash/Eq run on the borrowed caller key with no store-owned
        // caller value in flight, so a trait panic cannot unwind through a
        // caller destructor (the compound-hostile abort path). The detached
        // plan's Drop restores the map during any unwind.
        let hash = entries.hasher().hash_one(key);
        let RawEntryMut::Occupied(mut occupied) =
            entries.raw_entry_mut().from_key_hashed_nocheck(hash, key)
        else {
            return Ok(None);
        };
        let next = Self::committed_bytes_after_replace_v1(
            *committed_bytes,
            occupied.get().charged_bytes,
            0,
        )?;
        self.require_charge_shape(occupied.get(), "remove")?;
        if let Some(charge) = occupied.get_mut().charge.take() {
            let mut charges = [charge];
            if let Err(error) = self.accounting.release_many(&mut charges) {
                occupied.get_mut().charge = charges.into_iter().next();
                return Err(error);
            }
        }
        let (retired_key, removed) = occupied.remove_entry();
        *committed_bytes = next;
        self.restore_state(state);
        // Dropping an arbitrary client key may unwind. Publish the exact
        // ledger-matching postimage before invoking that destructor.
        // A panicking client key destructor must not prevent returning the
        // removed value or unwind into its destructor after the ledger refund.
        retire_client_value(retired_key);
        Ok(Some(removed.value))
    }

    fn sweep(&self, hint: SweepHint) -> Result<SweepStats, MemoryError> {
        let _permit = self.lifecycle.begin_mutation()?;
        let mut state = self.take_state()?;
        let mut reclaimed_items = 0usize;
        let mut reclaimed_bytes = 0u64;
        let mut selected = Vec::new();
        // Select newest-first without cloning client keys. All later charge
        // extraction/rollback is positional and callback-free.
        for (key, entry) in state.entries.iter().rev() {
            if reclaimed_bytes >= hint.target_bytes {
                break;
            }
            // A zero-byte entry frees no memory toward a BYTE target; evicting it is
            // pure data loss that never advances `reclaimed_bytes`. Skip it so a
            // positive target can never wipe an entire store of zero-byte entries
            // (over-eviction). It stays resident; only byte-bearing entries are swept.
            if entry.charged_bytes == 0 {
                continue;
            }
            reclaimed_items += 1;
            reclaimed_bytes = match reclaimed_bytes.checked_add(entry.charged_bytes) {
                Some(bytes) => bytes,
                None => {
                    self.restore_state(state);
                    return Err(MemoryError::InvalidState(
                        "retained-map sweep byte total overflowed committed accounting".to_string(),
                    ));
                }
            };
            // Hash only while the original charge still belongs to the entry.
            // A client panic here leaves the entire map and ledger untouched.
            selected.push((state.entries.hasher().hash_one(key), key as *const Key));
        }
        let next_committed_bytes =
            match Self::committed_bytes_after_replace_v1(state.committed_bytes, reclaimed_bytes, 0)
            {
                Ok(next) => next,
                Err(error) => {
                    self.restore_state(state);
                    return Err(error);
                }
            };
        let mut remaining = reclaimed_items;
        for entry in state.entries.values_mut().rev().filter(|entry| entry.charged_bytes > 0) {
            if remaining == 0 {
                break;
            }
            if let Err(error) = self.require_charge_shape(entry, "sweep") {
                for entry in state.entries.values_mut() {
                    entry.sweep_selected = false;
                }
                self.restore_state(state);
                return Err(error);
            }
            entry.sweep_selected = true;
            remaining -= 1;
        }
        // Validate exact hash-table identities before moving any charge. A
        // hash changed through interior mutability is a denied operation;
        // node addresses remain stable under subsequent removals.
        for &(hash, key_addr) in &selected {
            if state
                .entries
                .raw_entry()
                .from_hash(hash, |candidate| std::ptr::eq(candidate, key_addr))
                .is_none()
            {
                for entry in state.entries.values_mut() {
                    entry.sweep_selected = false;
                }
                self.restore_state(state);
                return Err(MemoryError::InvalidState(
                    "retained-map sweep key hash changed after insertion".to_string(),
                ));
            }
        }
        // Reserve before the refund; transfer into this vector is infallible.
        let mut retired_entries = Vec::with_capacity(selected.len());
        // Validate the entire plan before moving any non-clone charge out of an
        // entry. Otherwise a later corrupt candidate can drop an earlier lease
        // on the error path and partially refund the broker.
        let mut charges = Vec::new();
        for entry in state.entries.values_mut().filter(|entry| entry.sweep_selected) {
            if let Some(charge) = entry.charge.take() {
                charges.push(charge);
            }
        }
        if let Err(error) = self.accounting.release_many(&mut charges) {
            let mut charges = charges.into_iter();
            for entry in state.entries.values_mut().filter(|entry| entry.sweep_selected) {
                if entry.charge.is_none() {
                    entry.charge = charges.next();
                }
                entry.sweep_selected = false;
            }
            debug_assert!(charges.next().is_none());
            self.restore_state(state);
            return Err(error);
        }
        state.committed_bytes = next_committed_bytes;
        // The validated raw identities remove owned entries without invoking
        // client Hash/Eq/Drop after the refund. hashlink::retain is unsuitable:
        // its batch destructor can stop at the first client panic, leaking the
        // rest of the removed payloads and preventing a complete postimage.
        for (hash, key_addr) in selected {
            let RawEntryMut::Occupied(entry) = state
                .entries
                .raw_entry_mut()
                .from_hash(hash, |candidate| std::ptr::eq(candidate, key_addr))
            else {
                unreachable!("validated retained-map sweep identity disappeared")
            };
            retired_entries.push(entry.remove_entry());
        }
        self.restore_state(state);
        retire_map_entries(retired_entries);
        Ok(SweepStats { reclaimed_items, reclaimed_bytes })
    }

    fn stats_checked(&self) -> Result<StoreStats, MemoryError> {
        self.lifecycle.with_read_state(&self.state, |state| StoreStats {
            item_count: state.entries.len(),
            estimated_bytes: state.committed_bytes,
        })
    }

    fn keys_checked(&self) -> Result<Vec<Key>, MemoryError>
    where
        Key: Clone + Ord,
    {
        let mut keys = self.lifecycle.with_read_state(&self.state, |state| {
            state.entries.keys().map(|key| InFlight::new(key.clone())).collect::<Vec<_>>()
        })?;
        // Deterministic, key-sorted order independent of insertion order. The
        // sort runs outside the store boundary so a caller `Ord` panic cannot
        // hold or poison the state mutex.
        keys.sort_by(|a, b| a.get().cmp(b.get()));
        Ok(keys.iter_mut().map(InFlight::take).collect())
    }

    fn entries_checked(&self) -> Result<Vec<(Key, Value)>, MemoryError>
    where
        Key: Clone + Ord,
        Value: Clone,
    {
        let mut entries = self.lifecycle.with_read_state(&self.state, |state| {
            state
                .entries
                .iter()
                .map(|(key, entry)| {
                    let key = InFlight::new(key.clone());
                    let value = InFlight::new(entry.value.clone());
                    (key, value)
                })
                .collect::<Vec<_>>()
        })?;
        // Same deterministic key order as `keys`, outside the boundary.
        entries.sort_by(|a, b| a.0.get().cmp(b.0.get()));
        Ok(entries.iter_mut().map(|(key, value)| (key.take(), value.take())).collect())
    }

    fn close(&self) -> Result<(), MemoryError> {
        SimpleRetainedMapStore::close(self)
    }
}

// Owner tests use concise projections while still failing loudly on poison.
// These helpers are not part of the production/public API and never fabricate
// empty state from an authoritative checked error.
#[cfg(test)]
impl<Key, Value, Mode> SimpleRetainedMapStore<Key, Value, Mode>
where
    Key: Clone + Eq + Hash + ByteSized,
    Value: Clone + ByteSized,
{
    fn get(&self, key: &Key) -> Option<Value> {
        self.get_checked(key).expect("checked retained-map get")
    }

    fn stats(&self) -> StoreStats {
        self.stats_checked().expect("checked retained-map stats")
    }
}

#[cfg(test)]
impl<Key, Value, Mode> SimpleRetainedMapStore<Key, Value, Mode>
where
    Key: Clone + Eq + Hash + Ord + ByteSized,
    Value: Clone + ByteSized,
{
    fn keys(&self) -> Vec<Key> {
        self.keys_checked().expect("checked retained-map keys")
    }

    fn entries(&self) -> Vec<(Key, Value)> {
        self.entries_checked().expect("checked retained-map entries")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SingleTestCommand;
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, OnceLock, Weak};

    type UnbudgetedTestStore<Key, Value> = SimpleRetainedMapStore<Key, Value, TestUnbudgeted>;

    fn store() -> UnbudgetedTestStore<String, Vec<u8>> {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
        SimpleRetainedMapStore::new_unbudgeted_fixture(binding)
    }

    fn retained_cost(key: &str, payload_bytes: u64) -> u64 {
        // Independent oracle: key identity + value + hash link + entry state.
        payload_bytes + key.len() as u64 + 24 + 16
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct DeclaredBytesV1 {
        marker: u8,
        retained_bytes: u64,
    }

    impl ByteSized for DeclaredBytesV1 {
        fn byte_size(&self) -> u64 {
            self.retained_bytes
        }
    }

    fn declared_bytes_store_v1() -> UnbudgetedTestStore<String, DeclaredBytesV1> {
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "owner-map-declared-bytes".to_string(),
            None,
        );
        SimpleRetainedMapStore::new_unbudgeted_fixture(binding)
    }

    #[test]
    fn insert_commits_envelope_bytes() {
        let store = store();
        store.insert_checked("k1".to_string(), vec![1u8, 2, 3]).expect("insert");
        // The committed charge includes the retained key and index, not just the value.
        assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k1", 3));
        assert_eq!(
            store.stats(),
            StoreStats { item_count: 1, estimated_bytes: retained_cost("k1", 3) }
        );
        assert_eq!(store.get(&"k1".to_string()), Some(vec![1u8, 2, 3]));
    }

    #[test]
    fn committed_byte_overflow_rejects_before_payload_or_accounting_mutation_v1() {
        let store = declared_bytes_store_v1();
        let original = DeclaredBytesV1 { marker: 1, retained_bytes: 1 };
        store.insert_checked("stable".to_string(), original.clone()).expect("first insert");
        store.insert_checked("other".to_string(), original.clone()).expect("second insert");

        let replacement = DeclaredBytesV1 { marker: 9, retained_bytes: u64::MAX };
        let overwrite_error = store
            .insert_checked("stable".to_string(), replacement)
            .expect_err("overflowing overwrite must fail closed");
        assert!(matches!(overwrite_error, MemoryError::Capacity(_)));
        assert_eq!(store.get_checked(&"stable".to_string()).expect("stable read"), Some(original));
        assert_eq!(
            store.committed_bytes().expect("unchanged committed bytes"),
            retained_cost("stable", 1) + retained_cost("other", 1)
        );
        assert_eq!(store.stats_checked().expect("unchanged stats").item_count, 2);

        let max_store = declared_bytes_store_v1();
        max_store
            .insert_if_absent_checked(
                "max".to_string(),
                DeclaredBytesV1 { marker: 7, retained_bytes: u64::MAX - retained_cost("max", 0) },
            )
            .expect("the first maximum-sized declaration is representable");
        let insert_error = max_store
            .insert_if_absent_checked(
                "overflow".to_string(),
                DeclaredBytesV1 { marker: 8, retained_bytes: 1 },
            )
            .expect_err("overflowing vacant insert must fail closed");
        assert!(matches!(insert_error, MemoryError::Capacity(_)));
        assert_eq!(max_store.get_checked(&"overflow".to_string()).expect("absent read"), None);
        assert_eq!(max_store.committed_bytes().expect("maximum committed bytes"), u64::MAX);
        assert_eq!(max_store.stats_checked().expect("maximum stats").item_count, 1);
    }

    #[test]
    fn concurrent_insert_if_absent_has_one_creator_and_never_overwrites_v1() {
        const WRITERS: usize = 12;
        let store = Arc::new(store());
        let barrier = Arc::new(Barrier::new(WRITERS));
        let mut joins = Vec::new();
        for writer in 0..WRITERS {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            joins.push(std::thread::spawn(move || {
                barrier.wait();
                let writer_byte = u8::try_from(writer).expect("writer index fits u8");
                for _ in 0..10_000 {
                    match store.insert_if_absent_checked("canonical".to_string(), vec![writer_byte])
                    {
                        Ok(outcome) => return outcome,
                        Err(MemoryError::OperationInProgress(_)) => std::thread::yield_now(),
                        Err(error) => panic!("unexpected insert failure: {error}"),
                    }
                }
                panic!("mutation lane never became available")
            }));
        }
        let outcomes: Vec<_> =
            joins.into_iter().map(|join| join.join().expect("writer joins")).collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == RetainedMapInsertOutcomeV1::Inserted)
                .count(),
            1,
            "exactly one caller owns creation"
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == RetainedMapInsertOutcomeV1::AlreadyPresent)
                .count(),
            WRITERS - 1,
            "all losing callers observe an existing row"
        );
        assert_eq!(store.stats().item_count, 1);
        assert_eq!(
            store.committed_bytes().expect("committed bytes"),
            retained_cost("canonical", 1)
        );
    }

    #[test]
    fn concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1() {
        const READERS: usize = 4;
        const ITERATIONS: usize = 2_000;
        const PAYLOAD_BYTES: usize = 4_096;

        let store = Arc::new(store());
        let key = "concurrent-overwrite".to_string();
        let old = vec![0x11; PAYLOAD_BYTES];
        let new = vec![0xee; PAYLOAD_BYTES];
        store.insert_checked(key.clone(), old.clone()).expect("seed old payload");

        let barrier = Arc::new(Barrier::new(READERS + 1));
        let old_observations = Arc::new(AtomicUsize::new(0));
        let new_observations = Arc::new(AtomicUsize::new(0));
        let invalid_observations = Arc::new(AtomicUsize::new(0));
        let mut readers = Vec::new();
        for _ in 0..READERS {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let key = key.clone();
            let old = old.clone();
            let new = new.clone();
            let old_observations = Arc::clone(&old_observations);
            let new_observations = Arc::clone(&new_observations);
            let invalid_observations = Arc::clone(&invalid_observations);
            readers.push(std::thread::spawn(move || {
                for _ in 0..ITERATIONS {
                    // Reusable two-phase barrier: every read races exactly one
                    // overwrite epoch instead of merely starting near the writer.
                    barrier.wait();
                    let mut observed = None;
                    for _ in 0..10_000 {
                        match store.get_checked(&key) {
                            Err(MemoryError::OperationInProgress(_)) => {
                                std::thread::yield_now();
                            }
                            outcome => {
                                observed = Some(outcome);
                                break;
                            }
                        }
                    }
                    match observed.expect("checked read must leave the in-flight lane") {
                        Ok(Some(observed)) if observed == old => {
                            old_observations.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(Some(observed)) if observed == new => {
                            new_observations.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(Some(_)) | Ok(None) | Err(_) => {
                            invalid_observations.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    barrier.wait();
                }
            }));
        }

        let mut write_failures = 0usize;
        for iteration in 0..ITERATIONS {
            barrier.wait();
            let payload = if iteration % 2 == 0 { new.clone() } else { old.clone() };
            let mut published = false;
            for _ in 0..10_000 {
                match store.insert_checked(key.clone(), payload.clone()) {
                    Ok(()) => {
                        published = true;
                        break;
                    }
                    Err(MemoryError::OperationInProgress(_)) => std::thread::yield_now(),
                    Err(_) => break,
                }
            }
            if !published {
                write_failures += 1;
            }
            barrier.wait();
        }
        for reader in readers {
            reader.join().expect("reader joins without invalid observation");
        }

        assert_eq!(
            old_observations.load(Ordering::Relaxed) + new_observations.load(Ordering::Relaxed),
            READERS * ITERATIONS,
            "every checked read must classify as one complete published payload"
        );
        assert_eq!(write_failures, 0, "every synchronized overwrite must publish successfully");
        assert_eq!(
            invalid_observations.load(Ordering::Relaxed),
            0,
            "no checked read may observe absence, error, torn bytes, or a foreign generation"
        );
        assert!(
            old_observations.load(Ordering::Relaxed) > 0
                && new_observations.load(Ordering::Relaxed) > 0,
            "the synchronized overwrite epochs must expose both complete generations"
        );
    }

    #[derive(Clone, Debug)]
    struct PanickingHashKey {
        id: u8,
        armed: Arc<AtomicBool>,
    }

    impl ByteSized for PanickingHashKey {
        fn byte_size(&self) -> u64 {
            1
        }
    }

    impl PartialEq for PanickingHashKey {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }

    impl Eq for PanickingHashKey {}

    impl Hash for PanickingHashKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            assert!(!self.armed.load(Ordering::SeqCst), "injected hash unwind");
            self.id.hash(state);
        }
    }

    type ReentrantReadStore = SimpleRetainedMapStore<ReentrantReadKey, Vec<u8>, Budgeted>;

    #[derive(Clone)]
    struct ReentrantReadKey {
        id: u8,
        armed: Arc<AtomicBool>,
        store: Arc<OnceLock<Weak<ReentrantReadStore>>>,
        callbacks: Arc<AtomicUsize>,
    }
    impl ByteSized for ReentrantReadKey {
        fn byte_size(&self) -> u64 {
            1
        }
    }

    impl PartialEq for ReentrantReadKey {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }

    impl Eq for ReentrantReadKey {}

    impl Hash for ReentrantReadKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            if self.armed.load(Ordering::SeqCst) {
                let store = self.store.get().expect("store installed before read");
                let store = store.upgrade().expect("store remains live during read");
                assert!(
                    matches!(store.stats_checked(), Err(MemoryError::OperationInProgress(resource)) if resource == "retained map"),
                    "reentrant stats must report the held owner lock without blocking"
                );
                assert!(
                    matches!(store.close(), Err(MemoryError::OperationInProgress(resource)) if resource == "retained map"),
                    "reentrant close must roll back its closing phase without blocking"
                );
                self.callbacks.fetch_add(1, Ordering::SeqCst);
            }
            self.id.hash(state);
        }
    }

    /// A compound hostile caller type: panicking `Hash` AND panicking `Drop`.
    /// A store mutation that runs the trait while the value is in flight
    /// turns its own unwind into a double-panic process abort; these child
    /// scenarios pin that remove/insert must stay contained.
    #[derive(Clone)]
    struct CompoundHostileKey {
        name: String,
        hash_panics: Arc<AtomicBool>,
        drop_panics: Arc<AtomicBool>,
    }

    impl ByteSized for CompoundHostileKey {
        fn byte_size(&self) -> u64 {
            self.name.len() as u64
        }
    }

    impl PartialEq for CompoundHostileKey {
        fn eq(&self, other: &Self) -> bool {
            self.name == other.name
        }
    }

    impl Eq for CompoundHostileKey {}

    impl Hash for CompoundHostileKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            assert!(!self.hash_panics.load(Ordering::SeqCst), "armed compound-hostile Hash");
            self.name.hash(state);
        }
    }

    impl Drop for CompoundHostileKey {
        fn drop(&mut self) {
            assert!(!self.drop_panics.load(Ordering::SeqCst), "armed compound-hostile Drop");
        }
    }

    const COMPOUND_CHILD: &str = "QMP_RETAINED_MAP_COMPOUND_CHILD";

    fn compound_scenario_remove() {
        use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "owner-compound-hostile".to_string(),
            None,
        );
        let store: SimpleRetainedMapStore<CompoundHostileKey, Vec<u8>, Budgeted> =
            SimpleRetainedMapStore::new_budgeted(binding, broker);
        let hash_panics = Arc::new(AtomicBool::new(false));
        let drop_panics = Arc::new(AtomicBool::new(false));
        let key = CompoundHostileKey {
            name: "target".to_string(),
            hash_panics: Arc::clone(&hash_panics),
            drop_panics: Arc::clone(&drop_panics),
        };
        store.insert_checked(key.clone(), vec![1, 2, 3]).expect("seed while disarmed");

        hash_panics.store(true, Ordering::SeqCst);
        drop_panics.store(true, Ordering::SeqCst);
        let panicked =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.remove(&key)));
        assert!(panicked.is_err(), "armed Hash must panic inside remove");
        hash_panics.store(false, Ordering::SeqCst);
        drop_panics.store(false, Ordering::SeqCst);

        // The store (and the process) must remain usable with the exact
        // preimage still charged.
        assert_eq!(store.stats_checked().expect("stats after contained panic").item_count, 1);
        assert_eq!(store.get_checked(&key).expect("entry intact"), Some(vec![1, 2, 3]));
        store.close().expect("close after contained panic");
        let _ = key;
    }

    fn compound_scenario_insert() {
        use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "owner-compound-hostile-insert".to_string(),
            None,
        );
        let store: SimpleRetainedMapStore<CompoundHostileKey, Vec<u8>, Budgeted> =
            SimpleRetainedMapStore::new_budgeted(binding, broker);
        let hash_panics = Arc::new(AtomicBool::new(false));
        let drop_panics = Arc::new(AtomicBool::new(false));
        let key = CompoundHostileKey {
            name: "target".to_string(),
            hash_panics: Arc::clone(&hash_panics),
            drop_panics: Arc::clone(&drop_panics),
        };

        hash_panics.store(true, Ordering::SeqCst);
        drop_panics.store(true, Ordering::SeqCst);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.insert_checked(key, vec![9, 9])
        }));
        assert!(panicked.is_err(), "armed Hash must panic inside insert");
        hash_panics.store(false, Ordering::SeqCst);
        drop_panics.store(false, Ordering::SeqCst);

        assert_eq!(store.stats_checked().expect("stats after contained panic").item_count, 0);
        store.close().expect("close after contained panic");
    }

    #[test]
    fn compound_hostile_remove_panic_cannot_abort_process() {
        if std::env::var_os(COMPOUND_CHILD).is_some() {
            compound_scenario_remove();
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("retained_map::service::tests::compound_hostile_remove_panic_cannot_abort_process")
            .arg("--nocapture")
            .env(COMPOUND_CHILD, "remove")
            .require_one_test()
            .output()
            .expect("isolated compound-hostile remove child");
        assert!(
            output.status.success(),
            "a compound hostile caller type must not abort remove: status={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn compound_hostile_insert_panic_cannot_abort_process() {
        if std::env::var_os(COMPOUND_CHILD).is_some() {
            compound_scenario_insert();
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("retained_map::service::tests::compound_hostile_insert_panic_cannot_abort_process")
            .arg("--nocapture")
            .env(COMPOUND_CHILD, "insert")
            .require_one_test()
            .output()
            .expect("isolated compound-hostile insert child");
        assert!(
            output.status.success(),
            "a compound hostile caller type must not abort insert: status={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn read_hash_callback_reentry_is_typed_busy_and_preserves_exact_postimage() {
        use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};

        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "owner-read-reentry".to_string(),
            None,
        );
        let store = Arc::new(ReentrantReadStore::new_budgeted(binding, broker.clone()));
        let armed = Arc::new(AtomicBool::new(false));
        let callbacks = Arc::new(AtomicUsize::new(0));
        let owner = Arc::new(OnceLock::new());
        owner.set(Arc::downgrade(&store)).expect("install weak store once");
        let key = ReentrantReadKey {
            id: 7,
            armed: Arc::clone(&armed),
            store: owner,
            callbacks: Arc::clone(&callbacks),
        };
        store.insert_checked(key.clone(), vec![1, 2, 3]).expect("seed");
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            retained_cost("x", 3)
        );

        armed.store(true, Ordering::SeqCst);
        assert_eq!(store.get_checked(&key).expect("outer read"), Some(vec![1, 2, 3]));
        armed.store(false, Ordering::SeqCst);
        assert_eq!(callbacks.load(Ordering::SeqCst), 1, "lookup must invoke the client hash");
        assert_eq!(
            store.stats_checked().expect("published state"),
            StoreStats { item_count: 1, estimated_bytes: retained_cost("x", 3) }
        );
        assert_eq!(
            broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(),
            retained_cost("x", 3)
        );
        store.close().expect("closing phase restored after reentry");
        assert_eq!(broker.ledger_snapshot().expect("released ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn sweep_hash_callback_reentry_is_typed_busy_before_refund() {
        use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};

        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "owner-sweep-reentry".to_string(),
            None,
        );
        let store = Arc::new(ReentrantReadStore::new_budgeted(binding, broker.clone()));
        let armed = Arc::new(AtomicBool::new(false));
        let callbacks = Arc::new(AtomicUsize::new(0));
        let owner = Arc::new(OnceLock::new());
        owner.set(Arc::downgrade(&store)).expect("install weak store once");
        let key = ReentrantReadKey {
            id: 8,
            armed: Arc::clone(&armed),
            store: owner,
            callbacks: Arc::clone(&callbacks),
        };
        store.insert_checked(key, vec![1, 2, 3]).expect("charged preimage");
        assert_eq!(
            broker.ledger_snapshot().expect("preimage ledger").total_owner_bytes(),
            retained_cost("x", 3)
        );

        armed.store(true, Ordering::SeqCst);
        let swept = store.sweep(SweepHint { target_bytes: 3 }).expect("sweep succeeds");
        assert_eq!(swept.reclaimed_bytes, retained_cost("x", 3));
        assert_eq!(callbacks.load(Ordering::SeqCst), 1, "planning invokes client Hash once");
        assert_eq!(store.stats_checked().expect("complete postimage").item_count, 0);
        assert_eq!(broker.ledger_snapshot().expect("refunded ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn hash_unwind_during_detached_mutation_restores_owned_state_and_charge() {
        use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};

        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "owner-hash-unwind".to_string(),
            None,
        );
        let store: SimpleRetainedMapStore<PanickingHashKey, Vec<u8>, Budgeted> =
            SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
        let armed = Arc::new(AtomicBool::new(false));
        let key = PanickingHashKey { id: 1, armed: Arc::clone(&armed) };
        store.insert_checked(key.clone(), vec![1, 2, 3]).expect("seed");
        armed.store(true, Ordering::SeqCst);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.insert_if_absent_checked(
                PanickingHashKey { id: 2, armed: Arc::clone(&armed) },
                vec![9],
            );
        }));
        assert!(panic.is_err(), "hash callback must unwind after state extraction");
        armed.store(false, Ordering::SeqCst);

        assert_eq!(store.get_checked(&key).expect("restored read"), Some(vec![1, 2, 3]));
        assert_eq!(
            store.stats_checked().expect("restored stats").estimated_bytes,
            retained_cost("x", 3)
        );
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            retained_cost("x", 3)
        );
        store.close().expect("terminal release");
        assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
    }

    #[derive(Debug)]
    struct ClonePanicsWhenArmed {
        bytes: Vec<u8>,
        armed: Arc<AtomicBool>,
    }

    impl Clone for ClonePanicsWhenArmed {
        fn clone(&self) -> Self {
            assert!(
                !self.armed.load(Ordering::SeqCst),
                "injected clone panic while retained-map lock is held"
            );
            Self { bytes: self.bytes.clone(), armed: Arc::clone(&self.armed) }
        }
    }

    impl ByteSized for ClonePanicsWhenArmed {
        fn byte_size(&self) -> u64 {
            u64::try_from(self.bytes.len()).expect("test payload length fits u64")
        }
    }

    #[test]
    fn checked_reads_propagate_client_clone_panics_without_poisoning_owner_mutex_v1() {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-poison".to_string(), None);
        let store: UnbudgetedTestStore<String, ClonePanicsWhenArmed> =
            SimpleRetainedMapStore::new_unbudgeted_fixture(binding);
        let armed = Arc::new(AtomicBool::new(false));
        let key = "poison".to_string();
        store
            .insert_checked(
                key.clone(),
                ClonePanicsWhenArmed { bytes: vec![1, 2, 3], armed: Arc::clone(&armed) },
            )
            .expect("seed panic-capable payload");

        armed.store(true, Ordering::SeqCst);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.get_checked(&key);
        }));
        assert!(panic.is_err(), "clone panic must remain visible to the caller");
        armed.store(false, Ordering::SeqCst);

        assert!(
            store
                .get_checked(&key)
                .expect("owner lock remains usable after caller panic")
                .is_some(),
            "checked point read remains available after an isolated clone panic"
        );
        assert!(
            store.stats_checked().is_ok(),
            "checked stats must remain available after an isolated clone panic"
        );
        assert!(
            store.keys_checked().is_ok(),
            "checked key enumeration must remain available after an isolated clone panic"
        );
        assert!(
            store.entries_checked().is_ok(),
            "checked entry enumeration must remain available after an isolated clone panic"
        );

        armed.store(true, Ordering::SeqCst);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.entries_checked();
        }));
        assert!(panic.is_err(), "entry enumeration must propagate client clone panic");
        armed.store(false, Ordering::SeqCst);
        assert!(
            store.stats_checked().is_ok(),
            "entry-clone panic must release the read guard without poisoning the owner mutex"
        );
    }

    #[test]
    fn partial_entry_read_clone_and_drop_panics_cannot_abort() {
        const CHILD: &str = "QMP_RETAINED_PARTIAL_READ_CHILD";
        struct HostileReadValue {
            calls: Arc<AtomicUsize>,
            drop_panics: Arc<AtomicBool>,
        }
        impl Clone for HostileReadValue {
            fn clone(&self) -> Self {
                let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                assert_ne!(call, 2, "second value clone trap");
                Self { calls: Arc::clone(&self.calls), drop_panics: Arc::clone(&self.drop_panics) }
            }
        }
        impl ByteSized for HostileReadValue {
            fn byte_size(&self) -> u64 {
                1
            }
        }
        impl Drop for HostileReadValue {
            fn drop(&mut self) {
                assert!(!self.drop_panics.load(Ordering::SeqCst), "partial clone drop trap");
            }
        }
        if std::env::var_os(CHILD).is_some() {
            let binding = StoreBindingHandle::new(
                MemoryClass::MemoResidency,
                "owner:partial-read".into(),
                None,
            );
            let store: UnbudgetedTestStore<String, HostileReadValue> =
                SimpleRetainedMapStore::new_unbudgeted_fixture(binding);
            let calls = Arc::new(AtomicUsize::new(0));
            let drop_panics = Arc::new(AtomicBool::new(false));
            for key in ["a", "b"] {
                store
                    .insert_checked(
                        key.into(),
                        HostileReadValue {
                            calls: Arc::clone(&calls),
                            drop_panics: Arc::clone(&drop_panics),
                        },
                    )
                    .expect("seed");
            }
            drop_panics.store(true, Ordering::SeqCst);
            let caught =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.entries_checked()));
            assert!(caught.is_err(), "later Clone must panic");
            drop_panics.store(false, Ordering::SeqCst);
            assert_eq!(store.stats_checked().expect("read after unwind").item_count, 2);
            store.close().expect("close after unwind");
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("retained_map::service::tests::partial_entry_read_clone_and_drop_panics_cannot_abort")
            .env(CHILD, "1")
            .require_one_test()
.output()
            .expect("isolated partial-read child");
        assert!(
            output.status.success(),
            "partial read must not abort: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn put_get_remove_get_none_round_trip_corrects_bytes() {
        // scenario F: Simple* store put -> get -> remove -> get(None) round-trip
        // with byte-accounting correction at each step.
        let store = store();
        // get on an empty store is None and accounting is zero.
        assert_eq!(store.get(&"k".to_string()), None);
        assert_eq!(store.committed_bytes().expect("empty"), 0);

        store.insert_checked("k".to_string(), vec![1u8, 2, 3, 4]).expect("put");
        // get returns the stored value; bytes reflect exactly the value size.
        assert_eq!(store.get(&"k".to_string()), Some(vec![1u8, 2, 3, 4]));
        assert_eq!(store.committed_bytes().expect("after put"), retained_cost("k", 4));

        let removed = store.remove(&"k".to_string()).expect("remove");
        assert_eq!(removed, Some(vec![1u8, 2, 3, 4]), "remove returns the stored value");
        // After remove: get(None) and bytes corrected back to zero.
        assert_eq!(store.get(&"k".to_string()), None, "removed key must not be retrievable");
        assert_eq!(store.committed_bytes().expect("after remove"), 0);
        assert_eq!(store.stats(), StoreStats { item_count: 0, estimated_bytes: 0 });
    }

    // --- Audit U: equal keys need not be bit-identical. A second put of a key
    // that is equal under the lawful Eq + Hash contract must overwrite in place
    // and must NOT double-count bytes. ---

    #[derive(Clone, Debug)]
    struct CaseInsensitiveKey {
        // The canonical (lowercased) form drives equality; `raw` can differ for
        // two equal keys, exercising the Eq + Hash identity law.
        raw: String,
    }

    impl ByteSized for CaseInsensitiveKey {
        fn byte_size(&self) -> u64 {
            self.raw.len() as u64
        }
    }

    impl PartialEq for CaseInsensitiveKey {
        fn eq(&self, other: &Self) -> bool {
            self.raw.eq_ignore_ascii_case(&other.raw)
        }
    }

    impl Eq for CaseInsensitiveKey {}

    impl Hash for CaseInsensitiveKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.raw.len().hash(state);
            for byte in self.raw.bytes() {
                byte.to_ascii_lowercase().hash(state);
            }
        }
    }

    #[test]
    fn equal_hash_key_duplicate_overwrites_and_does_not_double_count() {
        // scenario F (audit U): two keys equal under Eq but not bit-identical
        // ("Key" vs "KEY") must collapse to one live entry with one live charge.
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "owner-partialeq".to_string(),
            None,
        );
        let store: UnbudgetedTestStore<CaseInsensitiveKey, Vec<u8>> =
            SimpleRetainedMapStore::new_unbudgeted_fixture(binding);

        store
            .insert_checked(CaseInsensitiveKey { raw: "Key".to_string() }, vec![0u8; 10])
            .expect("first put");
        assert_eq!(store.committed_bytes().expect("c1"), retained_cost("Key", 10));
        assert_eq!(store.stats().item_count, 1);

        // Re-put under an equal-but-not-identical key form: overwrite in place.
        store
            .insert_checked(CaseInsensitiveKey { raw: "KEY".to_string() }, vec![0u8; 3])
            .expect("overwrite put");
        assert_eq!(
            store.committed_bytes().expect("c2"),
            retained_cost("Key", 3),
            "Eq-equal duplicate must overwrite, never double-count bytes"
        );
        assert_eq!(store.stats().item_count, 1, "equal keys must collapse to one entry");
        // The new value is observable under either equal key form.
        assert_eq!(
            store.get(&CaseInsensitiveKey { raw: "key".to_string() }),
            Some(vec![0u8; 3]),
            "get under any equal key form returns the overwritten value"
        );
    }

    #[derive(Clone, Debug)]
    struct LookupProbeKey {
        id: u64,
        equality_probes: Arc<AtomicUsize>,
        hash_probes: Arc<AtomicUsize>,
    }

    impl ByteSized for LookupProbeKey {
        fn byte_size(&self) -> u64 {
            8
        }
    }

    impl PartialEq for LookupProbeKey {
        fn eq(&self, other: &Self) -> bool {
            self.equality_probes.fetch_add(1, Ordering::Relaxed);
            self.id == other.id
        }
    }

    impl Eq for LookupProbeKey {}

    impl Hash for LookupProbeKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.hash_probes.fetch_add(1, Ordering::Relaxed);
            self.id.hash(state);
        }
    }

    fn exercise_endpoint_point_operations_without_linear_probe_growth(endpoint_id: u64) {
        const LIVE_ENTRIES: u64 = 4_096;
        const MAX_EQUALITY_PROBES_PER_POINT_OPERATION: usize = 64;

        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            format!("owner-keyed-lookup-{endpoint_id}"),
            None,
        );
        let equality_probes = Arc::new(AtomicUsize::new(0));
        let hash_probes = Arc::new(AtomicUsize::new(0));
        let store: UnbudgetedTestStore<LookupProbeKey, Vec<u8>> =
            SimpleRetainedMapStore::new_unbudgeted_fixture(binding);

        for id in 0..LIVE_ENTRIES {
            store
                .insert_checked(
                    LookupProbeKey {
                        id,
                        equality_probes: Arc::clone(&equality_probes),
                        hash_probes: Arc::clone(&hash_probes),
                    },
                    vec![u8::try_from(id % 256).expect("modulo value fits u8")],
                )
                .expect("seed keyed entry");
        }

        equality_probes.store(0, Ordering::Relaxed);
        hash_probes.store(0, Ordering::Relaxed);
        let query = LookupProbeKey {
            id: endpoint_id,
            equality_probes: Arc::clone(&equality_probes),
            hash_probes: Arc::clone(&hash_probes),
        };
        let expected_byte = u8::try_from(endpoint_id % 256).expect("modulo value fits u8");
        assert_eq!(
            store.get_checked(&query).expect("checked endpoint lookup"),
            Some(vec![expected_byte])
        );
        assert!(
            equality_probes.load(Ordering::Relaxed) <= MAX_EQUALITY_PROBES_PER_POINT_OPERATION,
            "endpoint lookup must stay within the documented N-independent equality-probe bound"
        );
        assert_eq!(hash_probes.load(Ordering::Relaxed), 1, "lookup hashes the exact key once");

        equality_probes.store(0, Ordering::Relaxed);
        hash_probes.store(0, Ordering::Relaxed);
        store.insert_checked(query.clone(), vec![7u8; 3]).expect("overwrite exact key");
        assert!(
            equality_probes.load(Ordering::Relaxed) <= MAX_EQUALITY_PROBES_PER_POINT_OPERATION,
            "endpoint overwrite must stay within the documented N-independent equality-probe bound"
        );
        assert_eq!(hash_probes.load(Ordering::Relaxed), 1, "overwrite hashes the exact key once");

        equality_probes.store(0, Ordering::Relaxed);
        hash_probes.store(0, Ordering::Relaxed);
        assert_eq!(store.remove(&query).expect("remove exact key"), Some(vec![7u8; 3]));
        assert!(
            equality_probes.load(Ordering::Relaxed) <= MAX_EQUALITY_PROBES_PER_POINT_OPERATION,
            "endpoint remove must stay within the documented N-independent equality-probe bound"
        );
        assert_eq!(
            hash_probes.load(Ordering::Relaxed),
            1,
            "detached occupied entry validates and removes with one keyed probe"
        );
    }

    #[test]
    fn first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1() {
        exercise_endpoint_point_operations_without_linear_probe_growth(0);
        exercise_endpoint_point_operations_without_linear_probe_growth(4_095);
    }

    #[test]
    fn reinsert_same_key_recharges_to_new_size_not_doubled() {
        let store = store();
        store.insert_checked("k".to_string(), vec![0u8; 10]).expect("insert");
        assert_eq!(store.committed_bytes().expect("c1"), retained_cost("k", 10));
        // Replace with a smaller value: committed bytes track the live charge only,
        // never the sum of both inserts.
        store.insert_checked("k".to_string(), vec![0u8; 4]).expect("reinsert");
        assert_eq!(
            store.committed_bytes().expect("c2"),
            retained_cost("k", 4),
            "replacement must not double-charge the key"
        );
        assert_eq!(store.stats().item_count, 1);
    }

    type DropReentryStore = SimpleRetainedMapStore<String, DropReentryValue, Budgeted>;

    #[derive(Clone)]
    struct DropReentryValue {
        id: u8,
        bytes: u64,
        armed: Arc<AtomicBool>,
        home: Arc<OnceLock<Weak<DropReentryStore>>>,
        peer: Arc<OnceLock<Weak<DropReentryStore>>>,
        hits: Arc<AtomicUsize>,
    }

    impl ByteSized for DropReentryValue {
        fn byte_size(&self) -> u64 {
            self.bytes
        }
    }

    impl Drop for DropReentryValue {
        fn drop(&mut self) {
            if !self.armed.swap(false, Ordering::SeqCst) {
                return;
            }
            let home = self.home.get().expect("home installed").upgrade().expect("home live");
            assert!(matches!(
                home.stats_checked(),
                Err(MemoryError::OperationInProgress(resource)) if resource == "retained map"
            ));
            let peer = self.peer.get().expect("peer installed").upgrade().expect("peer live");
            peer.insert_checked(
                "peer".to_string(),
                Self {
                    id: 3,
                    bytes: 2,
                    armed: Arc::new(AtomicBool::new(false)),
                    home: Arc::clone(&self.home),
                    peer: Arc::clone(&self.peer),
                    hits: Arc::clone(&self.hits),
                },
            )
            .expect("other store can admit during retired value drop");
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn overwrite_drop_reentry_is_bounded_and_other_store_sees_exact_ledger() {
        use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};

        const CHILD: &str = "QMP_RETAINED_OVERWRITE_DROP_REENTRY_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let mut child =
                std::process::Command::new(std::env::current_exe().expect("test binary"))
                    .arg("overwrite_drop_reentry_is_bounded_and_other_store_sees_exact_ledger")
                    .env(CHILD, "1")
                    .require_one_test()
                    .spawn()
                    .expect("spawn bounded reentry test");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if let Some(status) = child.try_wait().expect("poll child") {
                    assert!(status.success(), "reentry child failed: {status}");
                    return;
                }
                if std::time::Instant::now() >= deadline {
                    child.kill().expect("terminate deadlocked child");
                    child.wait().expect("reap child");
                    panic!("reentry child exceeded bounded deadline");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let home = Arc::new(DropReentryStore::new_budgeted(
            StoreBindingHandle::new(MemoryClass::MemoResidency, "drop-home".into(), None),
            broker.clone(),
        ));
        let peer = Arc::new(DropReentryStore::new_budgeted(
            StoreBindingHandle::new(MemoryClass::MemoResidency, "drop-peer".into(), None),
            broker.clone(),
        ));
        let home_slot = Arc::new(OnceLock::new());
        let peer_slot = Arc::new(OnceLock::new());
        home_slot.set(Arc::downgrade(&home)).expect("install home");
        peer_slot.set(Arc::downgrade(&peer)).expect("install peer");
        let hits = Arc::new(AtomicUsize::new(0));
        let armed = Arc::new(AtomicBool::new(false));
        let value = |id, bytes, armed| DropReentryValue {
            id,
            bytes,
            armed,
            home: Arc::clone(&home_slot),
            peer: Arc::clone(&peer_slot),
            hits: Arc::clone(&hits),
        };
        home.insert_checked("home".to_string(), value(1, 4, Arc::clone(&armed))).expect("seed");
        armed.store(true, Ordering::SeqCst);
        home.insert_checked("home".to_string(), value(2, 7, Arc::new(AtomicBool::new(false))))
            .expect("overwrite and retire old value");

        assert_eq!(hits.load(Ordering::SeqCst), 1, "old value drop ran exactly once");
        assert_eq!(home.get_checked(&"home".to_string()).expect("home read").expect("row").id, 2);
        assert_eq!(peer.get_checked(&"peer".to_string()).expect("peer read").expect("row").id, 3);
        let expected_home = retained_cost("home", 7);
        let expected_peer = retained_cost("peer", 2);
        assert_eq!(home.stats_checked().expect("home stats").estimated_bytes, expected_home);
        assert_eq!(peer.stats_checked().expect("peer stats").estimated_bytes, expected_peer);
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            expected_home + expected_peer
        );
        home.close().expect("close home");
        peer.close().expect("close peer");
        assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn remove_releases_committed_bytes() {
        let store = store();
        store.insert_checked("k".to_string(), vec![9u8; 7]).expect("insert");
        let removed = store.remove(&"k".to_string()).expect("remove");
        assert_eq!(removed, Some(vec![9u8; 7]));
        // NEGATIVE-PAIR: removing the only entry must drop committed bytes to 0.
        assert_eq!(store.committed_bytes().expect("released"), 0);
        assert_eq!(store.stats().item_count, 0);
        // Removing again returns None and keeps accounting at 0.
        assert_eq!(store.remove(&"k".to_string()).expect("remove missing"), None);
        assert_eq!(store.committed_bytes().expect("still released"), 0);
    }

    #[test]
    fn sweep_reclaims_at_least_target_bytes_and_releases_them() {
        let store = store();
        store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
        store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
        store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
        assert_eq!(store.committed_bytes().expect("before"), 3 * retained_cost("a", 5));

        let stats =
            store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 }).expect("sweep");
        // Must reclaim until >= target: two retained entries cover a one-entry-plus-one target.
        assert_eq!(stats.reclaimed_items, 2);
        assert_eq!(stats.reclaimed_bytes, 2 * retained_cost("a", 5));
        assert_eq!(store.committed_bytes().expect("after sweep"), retained_cost("a", 5));
        assert_eq!(store.stats().item_count, 1);
    }

    #[test]
    fn overwrite_preserves_original_insertion_position_for_newest_first_sweep() {
        let store = store();
        store.insert_checked("a".to_string(), vec![1]).expect("a");
        store.insert_checked("b".to_string(), vec![2]).expect("b");
        store.insert_checked("c".to_string(), vec![3]).expect("c");

        // Updating b must not turn it into the newest entry.
        store.insert_checked("b".to_string(), vec![9, 9]).expect("overwrite b");
        let swept = store.sweep(SweepHint { target_bytes: 1 }).expect("sweep newest");

        assert_eq!(
            swept,
            SweepStats { reclaimed_items: 1, reclaimed_bytes: retained_cost("c", 1) }
        );
        assert_eq!(store.get(&"b".to_string()), Some(vec![9, 9]), "overwritten b stays resident");
        assert_eq!(store.get(&"c".to_string()), None, "original newest c is evicted first");
        assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn arbitrary_remove_preserves_remaining_newest_first_sweep_order() {
        let store = store();
        for key in ["a", "b", "c", "d"] {
            store.insert_checked(key.to_string(), vec![1]).expect("seed");
        }

        assert_eq!(store.remove(&"b".to_string()).expect("remove b"), Some(vec![1]));
        store.sweep(SweepHint { target_bytes: 1 }).expect("sweep d");
        assert_eq!(store.get(&"d".to_string()), None, "d remains newest after removing b");
        assert!(store.get(&"c".to_string()).is_some(), "c must survive the first sweep");

        store.sweep(SweepHint { target_bytes: 1 }).expect("sweep c");
        assert_eq!(store.get(&"c".to_string()), None, "c is next newest");
        assert_eq!(store.keys(), vec!["a".to_string()]);
    }

    #[test]
    fn byte_targeted_sweep_reclaims_full_cost_of_newest_entry() {
        // Empty values still retain keys and index state. A small byte target
        // must reclaim only the newest entry's full semantic cost.
        let store = store();
        store.insert_checked("a".to_string(), vec![]).expect("a"); // 0 bytes
        store.insert_checked("b".to_string(), vec![]).expect("b"); // 0 bytes
        store.insert_checked("c".to_string(), vec![0u8; 8]).expect("c"); // 8 bytes
        assert_eq!(
            store.committed_bytes().expect("before"),
            retained_cost("a", 0) + retained_cost("b", 0) + retained_cost("c", 8)
        );

        let stats = store.sweep(SweepHint { target_bytes: 6 }).expect("sweep");
        assert_eq!(stats.reclaimed_items, 1, "only the newest entry is swept");
        assert_eq!(stats.reclaimed_bytes, retained_cost("c", 8));
        assert_eq!(store.stats().item_count, 2, "older entries survive the byte sweep");
        assert!(
            store.get(&"a".to_string()).is_some() && store.get(&"b".to_string()).is_some(),
            "older entries are retained, not over-evicted"
        );
    }

    #[test]
    fn byte_targeted_sweep_of_empty_values_reclaims_key_and_index_cost() {
        // An empty value is not a free retained entry: key identity and index
        // state must be charged and returned by the sweep.
        let store = store();
        for k in ["a", "b", "c"] {
            store.insert_checked(k.to_string(), vec![]).expect("zero-byte insert");
        }
        let stats = store.sweep(SweepHint { target_bytes: 6 }).expect("sweep");
        assert_eq!(stats.reclaimed_items, 1);
        assert_eq!(stats.reclaimed_bytes, retained_cost("c", 0));
        assert_eq!(
            store.stats().item_count,
            2,
            "a positive byte target must remove only the byte-bearing entry needed"
        );
    }

    #[test]
    fn get_missing_key_is_none() {
        let store = store();
        assert_eq!(store.get(&"absent".to_string()), None);
    }

    #[test]
    fn keys_lists_inserted_keys_in_deterministic_sorted_order() {
        let store = store();
        // Insert OUT of sorted order to prove enumeration sorts, not echoes
        // insertion order.
        store.insert_checked("c".to_string(), vec![0u8; 1]).expect("c");
        store.insert_checked("a".to_string(), vec![0u8; 1]).expect("a");
        store.insert_checked("b".to_string(), vec![0u8; 1]).expect("b");
        assert_eq!(
            store.keys(),
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "keys must be returned in deterministic key-sorted order"
        );
    }

    #[test]
    fn entries_lists_pairs_in_same_sorted_order_as_keys() {
        let store = store();
        store.insert_checked("z".to_string(), vec![9u8]).expect("z");
        store.insert_checked("m".to_string(), vec![7u8, 7]).expect("m");
        let entries = store.entries();
        assert_eq!(
            entries,
            vec![("m".to_string(), vec![7u8, 7]), ("z".to_string(), vec![9u8])],
            "entries must be key-sorted and carry the stored value"
        );
        // The entry key order matches keys().
        let entry_keys: Vec<String> = entries.into_iter().map(|(k, _)| k).collect();
        assert_eq!(entry_keys, store.keys());
    }

    #[test]
    fn keys_after_remove_drops_the_removed_key() {
        let store = store();
        store.insert_checked("a".to_string(), vec![0u8]).expect("a");
        store.insert_checked("b".to_string(), vec![0u8]).expect("b");
        assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
        store.remove(&"a".to_string()).expect("remove a");
        // NEGATIVE-PAIR: a removed key must no longer enumerate.
        assert_eq!(store.keys(), vec!["b".to_string()]);
    }

    #[test]
    fn keys_on_empty_store_is_empty() {
        let store = store();
        assert!(store.keys().is_empty(), "empty store must enumerate no keys");
        assert!(store.entries().is_empty(), "empty store must enumerate no entries");
    }

    // --- Typed value: an arbitrary `Clone + ByteSized` value round-trips and the
    // broker accounting reflects the value's own byte size. ---

    #[derive(Clone, Debug, PartialEq)]
    struct TypedRecord {
        tag: u32,
        payload: Vec<u8>,
    }

    impl crate::shared::measure::ByteSized for TypedRecord {
        fn byte_size(&self) -> u64 {
            // 4 bytes for the tag + the payload length.
            4 + self.payload.len() as u64
        }
    }

    // --- F#3 completion: a CHARGED retained map routes its bytes through the broker
    // ledger (the accounting SSOT); the explicit uncharged mode does not. ---

    #[cfg(feature = "core-live-accounting")]
    #[test]
    fn uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot() {
        // scenario F#3 (explicit non-budgeted mode): a SimpleRetainedMapStore built
        // WITHOUT `charged_against` self-accounts; its committed bytes live only in
        // the store's own counter and are invisible to any broker ledger snapshot.
        // This is the intentional non-budgeted path, kept distinct from the charged
        // path below.
        use quanta_memory_platform_core::{
            ExecutionBudget, MemoryBudgetManifest, MemoryPreset, ResidencyPolicy,
        };

        let broker = quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
            .with_manifest(MemoryBudgetManifest::new(
                MemoryPreset::Production,
                ExecutionBudget::default(),
                ResidencyPolicy::new(1_000_000),
            ))
            .build()
            .expect("valid broker");

        // No `charged_against`: explicit non-budgeted mode.
        let store = store();
        store.insert_checked("k".to_string(), vec![0u8; 4096]).expect("insert");

        // The store sees its own bytes...
        assert_eq!(store.committed_bytes().expect("store bytes"), retained_cost("k", 4096));
        // ...but the broker ledger (the SSOT) sees none of them: an uncharged store
        // never wires its bytes into the ledger.
        assert_eq!(
            broker.ledger_snapshot().expect("snap").total_owner_bytes(),
            0,
            "an uncharged retained map must not route bytes through the broker ledger"
        );
    }

    #[test]
    fn typed_value_round_trips_and_accounts_its_own_byte_size() {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-typed".to_string(), None);
        let store: UnbudgetedTestStore<String, TypedRecord> =
            SimpleRetainedMapStore::new_unbudgeted_fixture(binding);
        let record = TypedRecord { tag: 7, payload: vec![1u8, 2, 3, 4, 5] };
        store.insert_checked("rec".to_string(), record.clone()).expect("insert typed");
        // get returns the exact typed value (no codec needed).
        assert_eq!(store.get(&"rec".to_string()), Some(record));
        // The value's declared bytes and retained key/index are both charged.
        assert_eq!(store.committed_bytes().expect("committed"), retained_cost("rec", 9));
        assert_eq!(
            store.stats(),
            StoreStats { item_count: 1, estimated_bytes: retained_cost("rec", 9) }
        );
    }

    // --- Opaque handle: `Arc<dyn Any + Send + Sync>` wrapped in MeasuredValue
    // (caller-supplied size) round-trips and is accounted by the broker. ---

    #[test]
    fn measured_any_handle_round_trips_and_accounts_caller_size() {
        use crate::shared::measure::MeasuredValue;
        use std::any::Any;
        use std::sync::Arc;

        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-handle".to_string(), None);
        let store: UnbudgetedTestStore<String, MeasuredValue<Arc<dyn Any + Send + Sync>>> =
            SimpleRetainedMapStore::new_unbudgeted_fixture(binding);

        let handle: Arc<dyn Any + Send + Sync> = Arc::new(vec![10u32, 20, 30]);
        // Caller computes the size accounting explicitly (4096 bytes here).
        store
            .insert_checked("h".to_string(), MeasuredValue::new(handle.clone(), 4096))
            .expect("insert handle");

        // get returns the same Arc; downcast proves identity of the carried value.
        let got = store.get(&"h".to_string()).expect("handle present");
        assert_eq!(got.size(), 4096, "wrapper carries the caller size");
        // Same Arc allocation (resume-state / whole-program-authority identity):
        // the retrieved erased Arc points at the original allocation.
        assert!(Arc::ptr_eq(&handle, got.value()), "retrieved handle must be the same Arc");
        let downcast = got.value().clone().downcast::<Vec<u32>>().expect("downcast to original");
        assert_eq!(*downcast, vec![10u32, 20, 30]);

        // Broker accounting reflects the caller-supplied size, not Arc::size_of.
        assert_eq!(store.committed_bytes().expect("committed"), retained_cost("h", 4096));
        assert_eq!(
            store.stats(),
            StoreStats { item_count: 1, estimated_bytes: retained_cost("h", 4096) }
        );

        // NEGATIVE-PAIR: removing the handle releases the caller-supplied charge.
        let removed =
            store.remove(&"h".to_string()).expect("remove handle").expect("handle present");
        assert_eq!(removed.size(), 4096);
        assert_eq!(store.committed_bytes().expect("released"), 0);
        assert!(store.get(&"h".to_string()).is_none(), "removed handle must not be retrievable");
    }

    // ---- #3: a charged retained map routes real bytes through the broker ledger ----
    #[cfg(feature = "core-live-accounting")]
    mod charged {
        use super::*;
        use quanta_memory_platform_core::{
            ExecutionBudget, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClassBudget,
            MemoryPreset, ReclaimPriority, ResidencyPolicy,
        };

        fn live_broker(class_hard: u64) -> LiveProcessMemoryBroker {
            let residency = ResidencyPolicy::new(10_000_000).with_class_budget(
                MemoryClass::MemoResidency,
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
        ) -> SimpleRetainedMapStore<String, Vec<u8>, Budgeted> {
            let binding =
                StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
            SimpleRetainedMapStore::new_budgeted(binding, broker.clone())
        }

        #[test]
        fn empty_value_with_huge_retained_key_is_denied_without_ledger_or_map_mutation() {
            let broker = live_broker(100);
            let store = charged(&broker);
            let huge_key = "k".repeat(1_024);
            assert!(matches!(
                store.insert_checked(huge_key.clone(), Vec::new()),
                Err(MemoryError::Capacity(_))
            ));
            assert_eq!(store.stats_checked().expect("unchanged stats"), StoreStats::default());
            assert_eq!(store.get_checked(&huge_key).expect("absent"), None);
            assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
        }

        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        struct PanicMeasuredKey(u8);

        impl ByteSized for PanicMeasuredKey {
            fn byte_size(&self) -> u64 {
                panic!("injected key measurement panic")
            }
        }

        #[test]
        fn key_measurement_panic_precedes_accounting_and_preserves_state() {
            let broker = live_broker(1_000);
            let binding = StoreBindingHandle::new(
                MemoryClass::MemoResidency,
                "key-measure-panic-owner".into(),
                None,
            );
            let store: SimpleRetainedMapStore<PanicMeasuredKey, Vec<u8>, Budgeted> =
                SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.insert_checked(PanicMeasuredKey(1), vec![1]);
            }));
            assert!(caught.is_err());
            assert_eq!(store.stats_checked().expect("state not poisoned"), StoreStats::default());
            assert_eq!(broker.ledger_snapshot().expect("no charge").total_owner_bytes(), 0);
        }

        #[test]
        fn counter_reads_show_open_busy_and_closed_images() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("key".to_string(), vec![1, 2, 3]).expect("insert");
            // Open image: the plain counters mirror the authoritative ledger charge.
            let charged = broker.ledger_snapshot().expect("open ledger").total_owner_bytes();
            assert!(charged > 0, "seed entry must hold a live charge");
            assert_eq!(
                store.stats_checked().expect("open stats"),
                StoreStats { item_count: 1, estimated_bytes: charged }
            );
            assert_eq!(store.committed_bytes().expect("open bytes"), charged);

            // Busy image: while the mutation lane is held, counter reads are
            // typed contention — never an empty success masking a detached plan.
            let permit = store.lifecycle.begin_mutation().expect("own lane");
            assert!(matches!(store.stats_checked(), Err(MemoryError::OperationInProgress(_))));
            assert!(matches!(store.committed_bytes(), Err(MemoryError::OperationInProgress(_))));
            drop(permit);

            // Closed image: the terminal empty image stays readable and consistent.
            store.close().expect("close");
            assert_eq!(store.stats_checked().expect("closed stats"), StoreStats::default());
            assert_eq!(store.committed_bytes().expect("closed bytes"), 0);
            assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
        }

        #[derive(Clone, Debug)]
        struct AliasKey {
            id: u8,
            declared_bytes: u64,
        }

        impl PartialEq for AliasKey {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }
        impl Eq for AliasKey {}
        impl Hash for AliasKey {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.id.hash(state);
            }
        }
        impl ByteSized for AliasKey {
            fn byte_size(&self) -> u64 {
                self.declared_bytes
            }
        }

        #[test]
        fn overwrite_preserves_retained_original_key_cost_and_exact_ledger() {
            let broker = live_broker(100);
            let binding =
                StoreBindingHandle::new(MemoryClass::MemoResidency, "alias-key-owner".into(), None);
            let store: SimpleRetainedMapStore<AliasKey, Vec<u8>, Budgeted> =
                SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
            let original = AliasKey { id: 1, declared_bytes: 2 };
            store.insert_checked(original.clone(), vec![1; 4]).expect("seed");
            let initial = 4 + 2 + 24 + 16;
            assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), initial);
            let equal_but_huge = AliasKey { id: 1, declared_bytes: 1_024 };
            store
                .insert_checked(equal_but_huge, vec![2; 6])
                .expect("original key remains retained");
            let expected = initial + 2;
            assert_eq!(store.committed_bytes().expect("store"), expected);
            assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);
            assert_eq!(store.get_checked(&original).expect("lookup"), Some(vec![2; 6]));
        }

        #[test]
        fn aggregate_cost_overflow_is_denied_before_new_entry_or_ledger_charge() {
            let broker = live_broker(1_000);
            let binding =
                StoreBindingHandle::new(MemoryClass::MemoResidency, "overflow-owner".into(), None);
            let store: SimpleRetainedMapStore<String, DeclaredBytesV1, Budgeted> =
                SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
            assert!(matches!(
                store.insert_checked(
                    "k".into(),
                    DeclaredBytesV1 { marker: 1, retained_bytes: u64::MAX }
                ),
                Err(MemoryError::Capacity(_))
            ));
            assert_eq!(store.stats_checked().expect("unchanged stats"), StoreStats::default());
            assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
        }

        #[derive(Clone, Debug)]
        struct PanicOnRetire {
            bytes: u64,
            panic_once: Arc<AtomicBool>,
        }

        impl ByteSized for PanicOnRetire {
            fn byte_size(&self) -> u64 {
                self.bytes
            }
        }

        impl Drop for PanicOnRetire {
            fn drop(&mut self) {
                if self.panic_once.swap(false, Ordering::SeqCst) {
                    panic!("client value destructor");
                }
            }
        }

        #[test]
        fn panic_in_old_value_drop_cannot_split_committed_bytes_from_ledger() {
            let broker = live_broker(1_000);
            let binding =
                StoreBindingHandle::new(MemoryClass::MemoResidency, "panic-owner".into(), None);
            let store: SimpleRetainedMapStore<String, PanicOnRetire, Budgeted> =
                SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
            let panic_once = Arc::new(AtomicBool::new(true));
            store
                .insert_checked(
                    "key".into(),
                    PanicOnRetire { bytes: 4, panic_once: Arc::clone(&panic_once) },
                )
                .expect("seed entry");

            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.insert_checked(
                    "key".into(),
                    PanicOnRetire { bytes: 8, panic_once: Arc::new(AtomicBool::new(false)) },
                )
            }));
            assert!(caught.is_err(), "client destructor did panic");
            assert_eq!(
                store.stats_checked().expect("complete postimage").estimated_bytes,
                retained_cost("key", 8)
            );
            assert_eq!(
                store.committed_bytes().expect("committed postimage"),
                retained_cost("key", 8)
            );
            assert_eq!(
                broker.ledger_snapshot().expect("ledger postimage").total_owner_bytes(),
                retained_cost("key", 8)
            );
            store.close().expect("close remains possible after client panic");
            assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
        }

        #[test]
        fn close_contains_client_destructor_panic_and_keeps_terminal_image() {
            let broker = live_broker(1_000);
            let binding = StoreBindingHandle::new(
                MemoryClass::MemoResidency,
                "close-panic-owner".into(),
                None,
            );
            let store: SimpleRetainedMapStore<String, PanicOnRetire, Budgeted> =
                SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
            store
                .insert_checked(
                    "key".into(),
                    PanicOnRetire { bytes: 9, panic_once: Arc::new(AtomicBool::new(true)) },
                )
                .expect("seed entry");
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.close()));
            assert!(caught.expect("client destructor must not escape close").is_ok());
            assert_eq!(store.stats_checked().expect("terminal image").item_count, 0);
            assert_eq!(store.committed_bytes().expect("terminal bytes"), 0);
            assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
            store.close().expect("double close is idempotent after client panic");
            assert!(matches!(
                store.insert_checked(
                    "late".into(),
                    PanicOnRetire { bytes: 1, panic_once: Arc::new(AtomicBool::new(false)) }
                ),
                Err(MemoryError::Closed(_))
            ));
        }

        #[test]
        fn drop_during_unwind_contains_two_client_destructor_panics_and_releases_all() {
            let broker = live_broker(1_000);
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let binding = StoreBindingHandle::new(
                    MemoryClass::MemoResidency,
                    "double-panic-owner".into(),
                    None,
                );
                let store: SimpleRetainedMapStore<String, PanicOnRetire, Budgeted> =
                    SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
                for key in ["a", "b"] {
                    store
                        .insert_checked(
                            key.into(),
                            PanicOnRetire { bytes: 3, panic_once: Arc::new(AtomicBool::new(true)) },
                        )
                        .expect("charged entry");
                }
                panic!("outer unwind");
            }));
            assert!(caught.is_err());
            assert_eq!(
                broker.ledger_snapshot().expect("all leases released").total_owner_bytes(),
                0
            );
        }

        #[derive(Clone, Debug)]
        struct PanicAfterSweepPlanKey {
            id: u64,
            hash_must_panic: Arc<AtomicBool>,
        }

        impl ByteSized for PanicAfterSweepPlanKey {
            fn byte_size(&self) -> u64 {
                8
            }
        }

        impl PartialEq for PanicAfterSweepPlanKey {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }

        impl Eq for PanicAfterSweepPlanKey {}

        impl Hash for PanicAfterSweepPlanKey {
            fn hash<H: Hasher>(&self, state: &mut H) {
                assert!(
                    !self.hash_must_panic.load(Ordering::SeqCst),
                    "post-plan client hash callback"
                );
                self.id.hash(state);
            }
        }

        #[derive(Clone, Debug)]
        struct AssertChargedDuringHashKey {
            id: u64,
            broker: LiveProcessMemoryBroker,
            armed: Arc<AtomicBool>,
            callbacks: Arc<AtomicUsize>,
        }

        impl ByteSized for AssertChargedDuringHashKey {
            fn byte_size(&self) -> u64 {
                8
            }
        }

        impl PartialEq for AssertChargedDuringHashKey {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }

        impl Eq for AssertChargedDuringHashKey {}

        impl Hash for AssertChargedDuringHashKey {
            fn hash<H: Hasher>(&self, state: &mut H) {
                if self.armed.load(Ordering::SeqCst) {
                    assert_eq!(
                        self.broker
                            .ledger_snapshot()
                            .expect("ledger in client Hash")
                            .total_owner_bytes(),
                        retained_cost("x1234567", 5),
                        "sweep must never call client Hash after refund"
                    );
                    self.callbacks.fetch_add(1, Ordering::SeqCst);
                }
                self.id.hash(state);
            }
        }

        #[test]
        fn sweep_never_hashes_client_key_after_refund() {
            let broker = live_broker(1_000);
            let binding = StoreBindingHandle::new(
                MemoryClass::MemoResidency,
                "sweep-hash-after-refund-owner".into(),
                None,
            );
            let store: SimpleRetainedMapStore<AssertChargedDuringHashKey, Vec<u8>, Budgeted> =
                SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
            let armed = Arc::new(AtomicBool::new(false));
            let callbacks = Arc::new(AtomicUsize::new(0));
            store
                .insert_checked(
                    AssertChargedDuringHashKey {
                        id: 1,
                        broker: broker.clone(),
                        armed: Arc::clone(&armed),
                        callbacks: Arc::clone(&callbacks),
                    },
                    vec![7; 5],
                )
                .expect("charged insert");
            armed.store(true, Ordering::SeqCst);
            let swept = store.sweep(SweepHint { target_bytes: 5 }).expect("sweep succeeds");
            assert_eq!(swept.reclaimed_bytes, retained_cost("x1234567", 5));
            assert_eq!(callbacks.load(Ordering::SeqCst), 1);
            assert_eq!(store.stats_checked().expect("postimage").item_count, 0);
            assert_eq!(broker.ledger_snapshot().expect("refunded ledger").total_owner_bytes(), 0);
        }

        #[derive(Clone, Debug)]
        struct PanicAfterSweepDropKey {
            id: &'static str,
            panic_once: Arc<AtomicBool>,
        }

        impl ByteSized for PanicAfterSweepDropKey {
            fn byte_size(&self) -> u64 {
                self.id.len() as u64
            }
        }

        impl PartialEq for PanicAfterSweepDropKey {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }

        impl Eq for PanicAfterSweepDropKey {}

        impl Hash for PanicAfterSweepDropKey {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.id.hash(state);
            }
        }

        impl Drop for PanicAfterSweepDropKey {
            fn drop(&mut self) {
                if self.panic_once.swap(false, Ordering::SeqCst) {
                    panic!("client key destructor");
                }
            }
        }

        #[test]
        fn sweep_hash_panic_before_refund_preserves_charged_preimage() {
            let broker = live_broker(1_000);
            let binding = StoreBindingHandle::new(
                MemoryClass::MemoResidency,
                "sweep-panic-owner".into(),
                None,
            );
            let store: SimpleRetainedMapStore<PanicAfterSweepPlanKey, Vec<u8>, Budgeted> =
                SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
            let hash_must_panic = Arc::new(AtomicBool::new(false));
            store
                .insert_checked(
                    PanicAfterSweepPlanKey { id: 1, hash_must_panic: Arc::clone(&hash_must_panic) },
                    vec![7; 5],
                )
                .expect("charged insert");
            hash_must_panic.store(true, Ordering::SeqCst);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.sweep(SweepHint { target_bytes: 5 })
            }));
            assert!(result.is_err(), "client Hash panic occurs before refund");
            assert_eq!(
                store.stats_checked().expect("charged preimage").estimated_bytes,
                retained_cost("x1234567", 5)
            );
            assert_eq!(
                broker.ledger_snapshot().expect("charged ledger").total_owner_bytes(),
                retained_cost("x1234567", 5)
            );
            hash_must_panic.store(false, Ordering::SeqCst);
            let stats = store.sweep(SweepHint { target_bytes: 5 }).expect("retry succeeds");
            assert_eq!(stats.reclaimed_bytes, retained_cost("x1234567", 5));
            assert_eq!(store.stats_checked().expect("postimage").estimated_bytes, 0);
            assert_eq!(broker.ledger_snapshot().expect("postimage ledger").total_owner_bytes(), 0);
        }

        #[test]
        fn panic_in_sweep_value_drop_is_contained_after_exact_refund() {
            let broker = live_broker(1_000);
            let binding = StoreBindingHandle::new(
                MemoryClass::MemoResidency,
                "sweep-value-panic-owner".into(),
                None,
            );
            let store: SimpleRetainedMapStore<String, PanicOnRetire, Budgeted> =
                SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
            store
                .insert_checked(
                    "key".into(),
                    PanicOnRetire { bytes: 5, panic_once: Arc::new(AtomicBool::new(true)) },
                )
                .expect("charged insert");
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.sweep(SweepHint { target_bytes: 5 })
            }));
            assert_eq!(
                result.expect("client destructor panic is contained").expect("sweep"),
                SweepStats { reclaimed_items: 1, reclaimed_bytes: retained_cost("key", 5) }
            );
            assert_eq!(store.stats_checked().expect("postimage").item_count, 0);
            assert_eq!(store.committed_bytes().expect("postimage bytes"), 0);
            assert_eq!(broker.ledger_snapshot().expect("postimage ledger").total_owner_bytes(), 0);
        }

        #[test]
        fn sweep_multiple_panicking_destructors_does_not_abort_process() {
            const CHILD: &str = "QMP_RETAINED_SWEEP_MULTI_PANIC_CHILD";
            if std::env::var_os(CHILD).is_some() {
                let broker = live_broker(1_000);
                let binding = StoreBindingHandle::new(
                    MemoryClass::MemoResidency,
                    "sweep-multi-panic-owner".into(),
                    None,
                );
                let store: SimpleRetainedMapStore<PanicAfterSweepDropKey, PanicOnRetire, Budgeted> =
                    SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
                let mut key_panics = Vec::new();
                let mut value_panics = Vec::new();
                for (key, bytes, panic) in [("survivor", 7, false), ("a", 5, true), ("b", 5, true)]
                {
                    let key_panic = Arc::new(AtomicBool::new(false));
                    let value_panic = Arc::new(AtomicBool::new(panic));
                    store
                        .insert_checked(
                            PanicAfterSweepDropKey { id: key, panic_once: Arc::clone(&key_panic) },
                            PanicOnRetire { bytes, panic_once: Arc::clone(&value_panic) },
                        )
                        .expect("seed charged entry");
                    key_panics.push(key_panic);
                    value_panics.push(value_panic);
                }
                key_panics[1].store(true, Ordering::SeqCst);
                key_panics[2].store(true, Ordering::SeqCst);
                struct SweepOnOuterUnwind<'a> {
                    store:
                        &'a SimpleRetainedMapStore<PanicAfterSweepDropKey, PanicOnRetire, Budgeted>,
                    result: &'a mut Option<Result<SweepStats, MemoryError>>,
                }

                impl Drop for SweepOnOuterUnwind<'_> {
                    fn drop(&mut self) {
                        *self.result = Some(
                            self.store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 }),
                        );
                    }
                }

                let mut result = None;
                let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _sweep = SweepOnOuterUnwind { store: &store, result: &mut result };
                    panic!("outer unwind before sweep destructor");
                }));
                assert!(outer.is_err(), "outer unwind reached sweep");
                let swept =
                    result.expect("sweep ran during outer unwind").expect("sweep completes");
                assert_eq!(swept.reclaimed_items, 2);
                assert_eq!(swept.reclaimed_bytes, retained_cost("a", 5) + retained_cost("b", 5));
                assert_eq!(store.stats_checked().expect("complete postimage").item_count, 1);
                assert_eq!(
                    store.committed_bytes().expect("complete postimage"),
                    retained_cost("survivor", 7)
                );
                assert_eq!(
                    broker.ledger_snapshot().expect("exact ledger").total_owner_bytes(),
                    retained_cost("survivor", 7)
                );
                assert!(key_panics[1..].iter().all(|flag| !flag.load(Ordering::SeqCst)));
                assert!(value_panics[1..].iter().all(|flag| !flag.load(Ordering::SeqCst)));
                store.close().expect("survivor remains closable");
                assert_eq!(
                    broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(),
                    0
                );
                return;
            }

            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .arg("--exact")
                .arg("retained_map::service::tests::charged::sweep_multiple_panicking_destructors_does_not_abort_process")
                .arg("--nocapture")
                .env(CHILD, "1")
                .require_one_test()
                .output()
                .expect("run isolated destructor-panic scenario");
            assert!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "sweep must not abort or unwind with two client Drop panics: status={:?}, stdout={}, stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        #[test]
        fn rebuild_denied_recharge_preserves_charged_preimage_v1() {
            let broker = live_broker(1_000);
            let store = charged(&broker);
            store.insert_checked("key".to_string(), vec![1u8; 30]).expect("seed charged entry");
            let rebuild_fence1 = broker.begin_rebuild().expect("begin rebuild");
            let error = store
                .insert_checked("key".to_string(), vec![2u8; 20])
                .expect_err("fenced recharge must fail closed");
            assert!(matches!(error, MemoryError::InvalidState(_)));
            assert_eq!(
                store.get_checked(&"key".to_string()).expect("checked read after recovery"),
                Some(vec![1u8; 30]),
                "managed recharge denial preserves the charged preimage"
            );
            assert_eq!(
                store.stats_checked().expect("checked stats").estimated_bytes,
                retained_cost("key", 30)
            );
            assert_eq!(
                broker.ledger_snapshot().expect("ledger after recovery").total_owner_bytes(),
                retained_cost("key", 30),
                "store and ledger retain the same charge authority"
            );
            assert!(rebuild_fence1.finish().expect("finish rebuild").drain_error.is_none());
        }

        #[test]
        fn charged_insert_routes_bytes_through_the_ledger() {
            // (a) charged mode: the ledger (SSOT) sees the map's real bytes, an
            // overwrite re-charges to the new size without double-counting, and
            // remove refunds the ledger.
            let broker = live_broker(1_000_000);
            let store = charged(&broker);

            store.insert_checked("k".to_string(), vec![0u8; 100]).expect("insert within cap");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("k", 100),
                "the ledger must see the charged map's bytes"
            );

            // Overwrite in place with a SMALLER value: the ledger tracks the live
            // charge only (40), never the sum of both inserts.
            store.insert_checked("k".to_string(), vec![0u8; 40]).expect("overwrite");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("k", 40),
                "an overwrite must re-charge to the new size, not double-count"
            );

            // remove refunds the live charge back to the ledger.
            store.remove(&"k".to_string()).expect("remove");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                0,
                "remove must refund the ledger"
            );
        }

        #[test]
        fn atomic_insert_if_absent_never_recharges_or_overwrites_an_existing_entry_v1() {
            let broker = live_broker(100);
            let store = charged(&broker);
            store.insert_checked("canonical".to_string(), vec![1u8; 30]).expect("seed");

            // The losing candidate is larger than the remaining budget. An
            // occupied decision must happen before charging, so it still returns
            // AlreadyPresent and leaves both authorities byte-for-byte intact.
            assert_eq!(
                store
                    .insert_if_absent_checked("canonical".to_string(), vec![2u8; 60])
                    .expect("occupied publication is not an admission attempt"),
                RetainedMapInsertOutcomeV1::AlreadyPresent
            );
            assert_eq!(
                store.get_checked(&"canonical".to_string()).expect("checked read"),
                Some(vec![1u8; 30]),
                "a losing publisher must not replace the canonical payload"
            );
            assert_eq!(
                store.stats_checked().expect("checked stats").estimated_bytes,
                retained_cost("canonical", 30)
            );
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("canonical", 30),
                "a losing publisher must not charge or refund the ledger"
            );
        }

        #[test]
        fn denied_atomic_insert_if_absent_publishes_neither_payload_nor_charge_v1() {
            let broker = live_broker(100);
            let store = charged(&broker);
            store.insert_checked("keep".to_string(), vec![0u8; 40]).expect("seed");
            let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
            let stats_before = store.stats_checked().expect("checked stats before");

            let denied = store.insert_if_absent_checked("denied".to_string(), vec![0u8; 20]);
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "a denied atomic publication must preserve the typed error: {denied:?}"
            );
            assert_eq!(store.stats_checked().expect("checked stats after"), stats_before);
            assert_eq!(
                store.get_checked(&"denied".to_string()).expect("checked absent read"),
                None,
                "charge denial must not publish a payload"
            );
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                ledger_before,
                "charge denial must not mutate the accounting authority"
            );
        }

        #[test]
        fn charged_insert_over_cap_is_denied_and_fails_closed() {
            // (b) over-cap insert is denied and fails closed: capture ledger + store
            // state before the denied op and assert both are byte-for-byte unchanged.
            let broker = live_broker(100); // MemoResidency hard cap 100 semantic bytes
            let store = charged(&broker);

            // Seed a surviving entry so the denied op cannot quietly drop it.
            store.insert_checked("keep".to_string(), vec![0u8; 40]).expect("seed within cap");
            let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
            let committed_before = store.committed_bytes().expect("committed before");
            let stats_before = store.stats();
            assert_eq!(ledger_before, retained_cost("keep", 40));

            // Both retained key/index charges make the combined image exceed the cap.
            let denied = store.insert_checked("big".to_string(), vec![0u8; 20]);
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "an over-cap insert must be a typed capacity error, got {denied:?}"
            );
            // Fail-closed: ledger, store counter, and stats all unchanged; the
            // surviving entry is intact.
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                ledger_before,
                "a denied insert must leave the ledger byte-for-byte unchanged"
            );
            assert_eq!(store.committed_bytes().expect("committed after"), committed_before);
            assert_eq!(store.stats(), stats_before);
            assert_eq!(store.get(&"keep".to_string()), Some(vec![0u8; 40]), "survivor intact");
        }

        #[test]
        fn charged_overwrite_denial_leaves_existing_entry_unchanged() {
            // (b, overwrite variant) a denied overwrite of an existing key must NOT
            // disturb the existing entry's value or its ledger charge — fail-closed
            // before any mutation or refund of the old charge.
            let broker = live_broker(100);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![1u8; 30]).expect("seed");
            let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
            assert_eq!(ledger_before, retained_cost("k", 30));

            // Re-insert the same key with a value that, charged BEFORE the old refund,
            // would exceed the semantic cap: must be denied.
            let denied = store.insert_checked("k".to_string(), vec![2u8; 60]);
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "over-cap overwrite must be denied, got {denied:?}"
            );
            // The old entry, its value, and its ledger charge survive untouched.
            assert_eq!(store.get(&"k".to_string()), Some(vec![1u8; 30]), "old value survives");
            assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k", 30));
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                ledger_before,
                "a denied overwrite must not refund the old charge"
            );
        }

        #[test]
        fn entry_remains_refundable_after_a_denied_overwrite() {
            // After a denied overwrite the entry must still hold a LIVE ledger
            // ticket (the original, or the broker-restored one) — never a stranded
            // non-live ticket. Proven end-to-end: a follow-up remove drains the
            // ledger to zero (a non-live ticket would be rejected and strand bytes).
            let broker = live_broker(100);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![1u8; 30]).expect("seed");

            let denied = store.insert_checked("k".to_string(), vec![2u8; 60]);
            assert!(matches!(denied, Err(MemoryError::Capacity(_))), "over-cap overwrite denied");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("k", 30)
            );

            // The surviving entry's ticket is refundable: removing it drains the ledger.
            let removed = store.remove(&"k".to_string()).expect("remove must succeed");
            assert_eq!(removed, Some(vec![1u8; 30]), "remove returns the surviving value");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                0,
                "the post-denial ticket was live and refundable (no strand)"
            );
        }

        #[test]
        fn charged_overwrite_during_rebuild_is_fail_closed_and_leaves_entry_intact() {
            // A charged overwrite while the broker is rebuilding must fail closed
            // (the recharge's release-old hits the rebuild gate), leaving the old
            // value and its ledger charge byte-for-byte intact — and the entry's
            // ticket still live so the value survives and is refundable after.
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![1u8; 40]).expect("seed");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("k", 40)
            );

            let rebuild_fence2 = broker.begin_rebuild().expect("begin rebuild");
            let denied = store.insert_checked("k".to_string(), vec![2u8; 20]);
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "overwrite during rebuild must be rejected, got {denied:?}"
            );
            assert_eq!(store.get(&"k".to_string()), Some(vec![1u8; 40]), "old value intact");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("k", 40)
            );

            // After rebuild the entry is still live: it overwrites and removes cleanly.
            assert!(rebuild_fence2.finish().expect("finish").drain_error.is_none());
            store.insert_checked("k".to_string(), vec![2u8; 20]).expect("overwrite resumes");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("k", 20)
            );
            store.remove(&"k".to_string()).expect("remove");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
        }

        #[test]
        fn charged_sweep_refunds_reclaimed_bytes_to_the_ledger() {
            // sweep must refund every reclaimed entry's bytes to the ledger.
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
            store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                3 * retained_cost("a", 5)
            );

            let stats =
                store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 }).expect("sweep");
            assert_eq!(stats.reclaimed_items, 2, "two retained entries cover the target");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("a", 5),
                "sweep must refund the reclaimed bytes to the ledger"
            );
        }

        #[test]
        fn charged_full_eviction_leaves_no_entry_key_ticket_or_accounting_residue_v1() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 2]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 3]).expect("b");
            store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
            let full_cost = retained_cost("a", 2) + retained_cost("b", 3) + retained_cost("c", 5);
            assert_eq!(broker.ledger_snapshot().expect("before").total_owner_bytes(), full_cost);

            let swept = store.sweep(SweepHint { target_bytes: full_cost }).expect("full sweep");
            assert_eq!(swept, SweepStats { reclaimed_items: 3, reclaimed_bytes: full_cost });
            assert_eq!(store.stats_checked().expect("checked empty stats"), StoreStats::default());
            assert!(store.keys().is_empty(), "full eviction leaves no reachable key");
            assert!(store.entries().is_empty(), "full eviction leaves no reachable entry");
            assert_eq!(store.committed_bytes().expect("empty committed bytes"), 0);
            assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
            assert!(
                store.state.lock().expect("state").entries.is_empty(),
                "the canonical owner contains no entry and therefore no live ticket"
            );
        }

        #[test]
        fn charged_remove_during_rebuild_is_fail_closed() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![0u8; 7]).expect("seed");
            let _rebuild_fence3 = broker.begin_rebuild().expect("begin rebuild");

            let denied = store.remove(&"k".to_string());
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "remove during rebuild must fail closed, got {denied:?}"
            );
            assert_eq!(
                store.get(&"k".to_string()),
                Some(vec![0u8; 7]),
                "entry must survive denial"
            );
            assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k", 7));
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("k", 7)
            );
        }

        #[test]
        fn charged_remove_rejects_a_ticketless_entry_without_mutating_state() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![0u8; 7]).expect("seed");
            let held = store
                .state
                .lock()
                .expect("state")
                .entries
                .get_mut("k")
                .expect("entry")
                .charge
                .take()
                .expect("budgeted charge");
            let denied = store.remove(&"k".to_string());
            assert!(matches!(denied, Err(MemoryError::InvalidState(_))), "got {denied:?}");
            assert_eq!(store.get(&"k".to_string()), Some(vec![0u8; 7]));
            assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k", 7));
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                retained_cost("k", 7)
            );
            store.state.lock().expect("state").entries.get_mut("k").expect("entry").charge =
                Some(held);
            store.close().expect("cleanup recovered charge");
            assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
        }

        #[test]
        fn charged_sweep_during_rebuild_is_fail_closed() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
            store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
            let rebuild_fence4 = broker.begin_rebuild().expect("begin rebuild");

            let denied = store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 });
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "sweep during rebuild must fail closed, got {denied:?}"
            );
            assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string(), "c".to_string()]);
            assert_eq!(store.committed_bytes().expect("committed"), 3 * retained_cost("a", 5));
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                3 * retained_cost("a", 5)
            );

            // A failed refund_many must also leave internal insertion order intact.
            // Once the accounting gate reopens, the original newest entry is still
            // the first one reclaimed.
            assert!(rebuild_fence4.finish().expect("finish rebuild").drain_error.is_none());
            let swept = store
                .sweep(SweepHint { target_bytes: retained_cost("c", 5) })
                .expect("sweep after retry");
            assert_eq!(
                swept,
                SweepStats { reclaimed_items: 1, reclaimed_bytes: retained_cost("c", 5) }
            );
            assert_eq!(store.get(&"c".to_string()), None, "c remains newest after denial");
            assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                2 * retained_cost("a", 5)
            );
        }

        #[test]
        fn charged_sweep_rejects_a_ticketless_candidate_without_partial_refund_or_eviction() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            for key in ["a", "b", "c"] {
                store.insert_checked(key.to_string(), vec![0u8; 5]).expect("seed");
            }
            let held = store
                .state
                .lock()
                .expect("state")
                .entries
                .get_mut("b")
                .expect("entry")
                .charge
                .take()
                .expect("budgeted charge");
            // c is visited first and has a ticket, then b exposes corruption.
            // No refund_many call or keyed removal may happen before all selected
            // entries have proven their ticket invariant.
            let denied = store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 });
            assert!(matches!(denied, Err(MemoryError::InvalidState(_))), "got {denied:?}");
            let insertion_order =
                store.state.lock().expect("state").entries.keys().cloned().collect::<Vec<_>>();
            assert_eq!(
                insertion_order,
                vec!["a".to_string(), "b".to_string(), "c".to_string()],
                "ticket validation failure must preserve canonical linked order"
            );
            assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string(), "c".to_string()]);
            assert_eq!(store.committed_bytes().expect("committed"), 3 * retained_cost("a", 5));
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                3 * retained_cost("a", 5)
            );
            store.state.lock().expect("state").entries.get_mut("b").expect("entry").charge =
                Some(held);
            store.close().expect("cleanup recovered charge");
            assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
        }

        #[test]
        fn uncharged_store_is_explicit_non_budgeted_mode() {
            // (c) no sink: the store self-accounts and never touches a ledger.
            let binding =
                StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
            let store: UnbudgetedTestStore<String, Vec<u8>> =
                SimpleRetainedMapStore::new_unbudgeted_fixture(binding);
            store.insert_checked("k".to_string(), vec![0u8; 7]).expect("insert");
            assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k", 7));
        }

        #[test]
        fn remove_contains_panicking_key_and_returns_value_after_exact_refund() {
            use std::hash::{Hash, Hasher};
            use std::sync::atomic::{AtomicUsize, Ordering};

            static PANIC_COUNT: AtomicUsize = AtomicUsize::new(0);

            #[derive(Debug)]
            struct Key {
                text: String,
                panic_on_drop: bool,
            }

            impl Clone for Key {
                fn clone(&self) -> Self {
                    Self { text: self.text.clone(), panic_on_drop: false }
                }
            }

            impl PartialEq for Key {
                fn eq(&self, other: &Self) -> bool {
                    self.text == other.text
                }
            }

            impl Eq for Key {}

            impl Hash for Key {
                fn hash<H: Hasher>(&self, state: &mut H) {
                    self.text.hash(state);
                }
            }

            impl ByteSized for Key {
                fn byte_size(&self) -> u64 {
                    self.text.len() as u64
                }
            }

            impl Drop for Key {
                fn drop(&mut self) {
                    if self.panic_on_drop {
                        PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
                        panic!("retired key drop trap");
                    }
                }
            }

            #[derive(Clone, Debug)]
            struct Value;

            impl ByteSized for Value {
                fn byte_size(&self) -> u64 {
                    1
                }
            }

            impl Drop for Value {
                fn drop(&mut self) {
                    PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
                    panic!("returned value drop trap");
                }
            }

            const CHILD: &str = "QMP_RETAINED_REMOVE_DROP_CHILD";
            if std::env::var_os(CHILD).is_some() {
                let broker = live_broker(1_000_000);
                let binding = StoreBindingHandle::new(
                    MemoryClass::MemoResidency,
                    "remove-drop-test".into(),
                    None,
                );
                let store = SimpleRetainedMapStore::<Key, Value, Budgeted>::new_budgeted(
                    binding,
                    broker.clone(),
                );
                store
                    .insert_checked(Key { text: "key".into(), panic_on_drop: true }, Value)
                    .expect("seed");
                let query = Key { text: "key".into(), panic_on_drop: false };
                let returned = store.remove(&query).expect("remove").expect("returned value");
                assert_eq!(store.stats_checked().expect("postimage").item_count, 0);
                assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
                retire_client_value(returned);
                assert_eq!(PANIC_COUNT.load(Ordering::SeqCst), 2, "both fields retired");
                return;
            }

            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .arg("remove_contains_panicking_key_and_returns_value_after_exact_refund")
                .arg("--nocapture")
                .env(CHILD, "1")
                .require_one_test()
                .output()
                .expect("spawn child");
            assert!(
                output.status.success(),
                "child {:?}; stderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
