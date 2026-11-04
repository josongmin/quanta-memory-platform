mod cases_01;
mod cases_02;
mod read_admission_v73;

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
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.remove(&key)));
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

// --- Opaque handle: `Arc<dyn Any + Send + Sync>` wrapped in MeasuredValue
// (caller-supplied size) round-trips and is accounted by the broker. ---

// ---- #3: a charged retained map routes real bytes through the broker ledger ----
#[cfg(feature = "core-live-accounting")]
mod charged;
