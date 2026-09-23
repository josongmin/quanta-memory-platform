pub mod backings;
pub mod service;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(feature = "spill-redb")]
use quanta_memory_platform_core::MemoryError;
use quanta_memory_platform_core::MemoryResult;

pub use service::{
    DurableRestoreTarget, DurableSpillTarget, InMemorySpillTarget, InProcessSpillTarget,
    NoopSpillController, RetainingSpillTarget, RoutedSpillController, SpillFailure, SpillInventory,
    SpillReceipt, SpillTarget,
};

pub trait SpillController<Handle> {
    /// Offload a handle to the spill tier. On failure the caller regains the
    /// original handle and remains responsible for its accounting authority.
    fn spill(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>>;

    /// Restore a previously spilled handle (oldest first), or `None` if the tier
    /// holds nothing to bring back. Default: nothing to restore (a write-only /
    /// no-op controller).
    fn restore(&self) -> MemoryResult<Option<Handle>> {
        Ok(None)
    }
}

/// Marker contract for controllers whose successful receipt keeps the exact
/// handle resident in this process. It is the only controller class accepted
/// by charge-preserving revocable-store transfer.
///
/// External controllers cannot claim this accounting-bearing capability:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{InProcessSpillController, SpillController};
/// struct Foreign;
/// impl SpillController<u8> for Foreign {
///     fn spill(&self, handle: u8) -> Result<quanta_memory_platform_facade::SpillReceipt, quanta_memory_platform_facade::SpillFailure<u8>> {
///         unimplemented!()
///     }
/// }
/// impl InProcessSpillController<u8> for Foreign {}
/// ```
pub trait InProcessSpillController<Handle>:
    SpillController<Handle> + service::sealed::InProcessSpillController<Handle>
{
}

/// Marker contract for controllers that commit an encoded handle to a durable
/// tier before returning [`SpillReceipt::Durable`].
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{DurableSpillController, SpillController};
/// struct Foreign;
/// impl SpillController<Vec<u8>> for Foreign {
///     fn spill(&self, handle: Vec<u8>) -> Result<quanta_memory_platform_facade::SpillReceipt, quanta_memory_platform_facade::SpillFailure<Vec<u8>>> {
///         unimplemented!()
///     }
/// }
/// impl DurableSpillController<Vec<u8>> for Foreign {}
/// ```
pub trait DurableSpillController<Handle>:
    SpillController<Handle> + service::sealed::DurableSpillController<Handle>
{
}

/// Persisted, never-reused identity for one durable write attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DurableRecordId(u64);

impl DurableRecordId {
    #[must_use]
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Read-side resolution of an ambiguous durable write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableRecordResolution {
    Present {
        inventory_after: u64,
    },
    /// The record existed but was durably consumed. A stale write must not
    /// recreate it; a restore owned by the matching claim may publish locally.
    Finalized,
    /// The reserved record never became durable.
    Absent,
    /// The record was durably consumed by a DIFFERENT restore claim than the
    /// one presenting for reconciliation. Only a restore reconcile can produce
    /// this: the presenting claimant's fenced provisional copy must be retired,
    /// never published. The write path never observes this variant.
    Superseded,
}

/// Durable write failure with an explicit authority disposition.
#[derive(Debug)]
pub enum DurableWriteFailure {
    /// The record is known not to have committed; source authority remains live.
    NotCommitted(Box<quanta_memory_platform_core::MemoryError>),
    /// Commit acknowledgement and the follow-up probe both failed. The source
    /// must be fenced until [`ReconciledDurableSpillController::resolve_record`]
    /// determines the persisted postimage.
    RecoveryRequired {
        record_id: DurableRecordId,
        commit_error: Box<quanta_memory_platform_core::MemoryError>,
        probe_error: Box<quanta_memory_platform_core::MemoryError>,
    },
}

/// Durable write protocol used by accounting-bearing stores. Bytes are borrowed,
/// so a controller panic or precommit error cannot consume the source's only
/// recovery copy.
pub trait ReconciledDurableSpillController:
    service::sealed::DurableSpillController<Vec<u8>>
{
    fn reserve_record_id(&self) -> MemoryResult<DurableRecordId>;
    fn commit_spill(
        &self,
        record_id: DurableRecordId,
        bytes: &[u8],
    ) -> Result<SpillReceipt, DurableWriteFailure>;
    fn resolve_record(&self, record_id: DurableRecordId) -> MemoryResult<DurableRecordResolution>;
}

/// Unforgeable single-owner lease for one durable restore attempt (FUP-002).
///
/// A restore candidate's bare sequence is stable across restarts and retries,
/// so it cannot identify WHO may consume the record. Each `prepare` or
/// recovery issues a fresh claim binding the backend identity, the sequence,
/// and a persisted monotonic generation. Every authority-changing operation
/// (`commit` / `finalize` / `abort` / `reconcile`) validates the presented
/// claim against the backend's persisted active claimant; a stale or foreign
/// claim is rejected, so exactly one claimant can publish each record.
///
/// The fields are private and there is no public constructor: only the
/// backend that issued a claim can mint it, and only holders of a live
/// candidate can present it. A bare sequence can no longer authorize a
/// permission change:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::DurableRestoreClaim;
/// let forged = DurableRestoreClaim { backend: 0, generation: 0, sequence: 0 };
/// ```
///
/// Claims are deliberately non-`Clone` and non-`Copy`; duplicating the token
/// would let one holder create multiple independently-owned restore flows:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::DurableRestoreClaim;
/// fn require_clone<T: Clone>() {}
/// require_clone::<DurableRestoreClaim>();
/// ```
///
/// ```compile_fail
/// use quanta_memory_platform_facade::DurableRestoreClaim;
/// fn require_copy<T: Copy>() {}
/// require_copy::<DurableRestoreClaim>();
/// ```
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct DurableRestoreClaim {
    backend: u128,
    generation: u64,
    sequence: u64,
}

impl DurableRestoreClaim {
    /// Backend-issued claim. Only crate backends mint claims; callers receive
    /// them exclusively through a live restore candidate.
    #[cfg(any(feature = "spill-redb", all(test, feature = "revocable_store")))]
    pub(crate) fn issued(backend: u128, generation: u64, sequence: u64) -> Self {
        Self { backend, generation, sequence }
    }

    /// Test-only claim for in-crate fake targets. Real backends never accept a
    /// claim whose backend identity they did not persist themselves.
    #[cfg(all(test, feature = "revocable_store"))]
    pub(crate) fn for_test(backend: u128, generation: u64, sequence: u64) -> Self {
        Self::issued(backend, generation, sequence)
    }

    /// Persisted identity of the backend file that issued this claim. Stable
    /// across renames and reopens; distinct files hold distinct identities.
    #[must_use]
    pub fn backend(&self) -> u128 {
        self.backend
    }

    /// Per-backend monotonic attempt number. Only the latest generation for an
    /// active sequence carries authority; older generations are stale.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Durable sequence this claim authorizes. Numeric sequences collide across
    /// files, so the backend always pairs it with [`Self::backend`].
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Non-destructive durable restore candidate protected by a persisted claim.
/// The destination remains authoritative until
/// [`DurableRestoreController::commit_restore`] succeeds. It is not cloneable:
/// move it through the restore flow and borrow its claim for retries.
///
/// ```compile_fail
/// use quanta_memory_platform_facade::DurableRestoreCandidate;
/// fn require_clone<T: Clone>() {}
/// require_clone::<DurableRestoreCandidate>();
/// ```
#[derive(Debug)]
pub struct DurableRestoreCandidate {
    pub sequence: u64,
    bytes: Vec<u8>,
    /// Single-owner lease authorizing this candidate's permission changes.
    /// Mirrors `sequence`; the backend validates the claim, never the bare id.
    pub claim: DurableRestoreClaim,
    /// Keeps the backing-owned restore staging bytes charged until this
    /// candidate is retired. The caller's own copies and codec allocations
    /// remain the caller's responsibility.
    _read_lease: Option<RestoreReadLease>,
}

impl DurableRestoreCandidate {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[cfg(feature = "spill-redb")]
    pub(crate) fn from_redb(
        sequence: u64,
        bytes: Vec<u8>,
        claim: DurableRestoreClaim,
        lease: RestoreReadLease,
    ) -> Self {
        Self { sequence, bytes, claim, _read_lease: Some(lease) }
    }

    /// Test-only candidate for in-crate fake targets.
    #[cfg(all(test, feature = "revocable_store"))]
    pub(crate) fn for_test(sequence: u64, bytes: Vec<u8>, claim: DurableRestoreClaim) -> Self {
        Self { sequence, bytes, claim, _read_lease: None }
    }
}

impl PartialEq for DurableRestoreCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.sequence == other.sequence && self.bytes == other.bytes && self.claim == other.claim
    }
}

impl Eq for DurableRestoreCandidate {}

/// Tracks only byte buffers allocated and retained by this backing. It cannot
/// meter arbitrary caller copies or private codec scratch.
#[derive(Debug)]
pub(crate) struct RestoreReadLease {
    used: Arc<AtomicU64>,
    bytes: u64,
}

impl RestoreReadLease {
    #[cfg(feature = "spill-redb")]
    pub(crate) fn acquire(used: Arc<AtomicU64>, bytes: u64, max: u64) -> MemoryResult<Self> {
        let mut current = used.load(Ordering::Acquire);
        loop {
            let next = current
                .checked_add(bytes)
                .ok_or_else(|| MemoryError::capacity("redb restore staging bytes exceed u64"))?;
            if next > max {
                return Err(MemoryError::capacity(format!(
                    "redb restore staging requires {next} bytes, above configured {max}-byte in-flight limit"
                )));
            }
            match used.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Ok(Self { used, bytes }),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for RestoreReadLease {
    fn drop(&mut self) {
        let prior = self.used.fetch_sub(self.bytes, Ordering::AcqRel);
        debug_assert!(prior >= self.bytes, "restore staging lease underflow");
    }
}

/// Persisted phase of an interrupted durable restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableRestorePhase {
    /// The record is still in the ordinary durable queue under an active claim.
    Prepared,
    /// The record was removed from the queue but its bytes remain in the
    /// durable recovery slot until resident publication is acknowledged.
    Committed,
}

/// Restart recovery material for the single persisted restore claim.
/// Recovery material is also non-cloneable because it carries the claim. Move
/// it into the resumed restore flow; retry operations by borrowing the claim.
///
/// ```compile_fail
/// use quanta_memory_platform_facade::DurableRestoreRecovery;
/// fn require_clone<T: Clone>() {}
/// require_clone::<DurableRestoreRecovery>();
/// ```
#[derive(Debug, Eq, PartialEq)]
pub struct DurableRestoreRecovery {
    pub candidate: DurableRestoreCandidate,
    pub phase: DurableRestorePhase,
}

/// Classification of a failed durable-delete commit.
#[derive(Debug)]
pub enum DurableCommitFailure {
    /// The durable record is known to remain authoritative.
    Retained(Box<quanta_memory_platform_core::MemoryError>),
    /// The backend cannot prove whether the exact record was deleted.
    Ambiguous(Box<quanta_memory_platform_core::MemoryError>),
}

impl DurableCommitFailure {
    pub fn retained(error: quanta_memory_platform_core::MemoryError) -> Self {
        Self::Retained(Box::new(error))
    }

    pub fn ambiguous(error: quanta_memory_platform_core::MemoryError) -> Self {
        Self::Ambiguous(Box::new(error))
    }
}

/// Result of a durable-to-source transfer.
#[derive(Debug)]
#[must_use = "ambiguous restore outcomes require sequence reconciliation"]
pub enum DurableRestoreOutcome {
    Empty,
    Restored { sequence: u64 },
    RecoveryRequired { sequence: u64, commit_error: Box<quanta_memory_platform_core::MemoryError> },
}

pub trait DurableRestoreController: DurableSpillController<Vec<u8>> {
    fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>>;
    /// Move exactly the claimed record into the recovery slot. The presented
    /// claim must be the backend's current active claimant; stale or foreign
    /// claims are rejected without touching the record.
    fn commit_restore(&self, claim: &DurableRestoreClaim) -> Result<(), DurableCommitFailure>;
    /// Delete the durable recovery payload only after the resident copy has a
    /// live charge and is ready for publication. Idempotent only for the same
    /// claimant's retry; any other claimant must present its own live claim.
    fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()>;
    /// Release a prepared claim when decode or destination admission fails.
    fn abort_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()>;
    /// Resolve an ambiguous exact-delete outcome and release a retained claim.
    /// A record consumed by a different claimant resolves
    /// [`DurableRecordResolution::Superseded`] so the presenting claimant
    /// retires its provisional copy instead of publishing it.
    fn reconcile_restore(
        &self,
        claim: &DurableRestoreClaim,
    ) -> MemoryResult<DurableRecordResolution>;
    /// Persisted claim left by an interrupted or ambiguous restore.
    fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>>;
    /// Claim the persisted restore for this caller and read its recovery
    /// material without changing its phase. Each call mints a fresh claim
    /// generation; older holders of the same sequence lose authority.
    fn recover_active_restore(&self) -> MemoryResult<Option<DurableRestoreRecovery>>;
}
