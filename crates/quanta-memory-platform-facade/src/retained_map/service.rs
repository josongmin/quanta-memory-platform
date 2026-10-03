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
mod tests;
