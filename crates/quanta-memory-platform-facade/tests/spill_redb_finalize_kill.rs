//! Real finalize instruction-point kill rail for the durable spill tier
//! (DURABILITY-001 L1).
//!
//! Boundary proved here: the child process is ended with
//! [`std::process::Child::kill`] — `SIGKILL` on Unix, which runs no
//! destructors and flushes nothing — while blocked at a NAMED point inside
//! `RedbSpillBacking::finalize_restore`, and a survivor reopens the SAME
//! database file cold. The two points bracket the finalize linearization
//! point: staged-but-uncommitted (`FinalizeBeforeCommit`) and
//! commit-returned-`Ok` (`FinalizeAfterCommit`).
//!
//! Boundary NOT proved here: anything inside dependency-owned `pwrite`/`fsync`
//! (L2), and power loss (L3). A `SIGKILL` rail cannot fail the storage device
//! mid-sector-write, so power-loss durability stays `UNVERIFIED`.
//!
//! Every test drives recovery through the public
//! [`ReconciledDurableSpillTarget`] / [`DurableRestoreTarget`] surface only.
//! The child installs exactly one test-only hook (feature
//! `durability-test-hooks`, absent from `default`/`full`/`full-backings`);
//! default builds compile the existing finalize path with zero added code.
//!
//! Run with:
//!
//! ```bash
//! cargo test --locked -p quanta-memory-platform-facade \
//!     --features spill-redb,durability-test-hooks \
//!     --test spill_redb_finalize_kill
//!
//! # Bounded opt-in stress: fresh database per iteration, same oracles.
//! QMP_FINALIZE_KILL_ITERATIONS=20 cargo test --locked \
//!     -p quanta-memory-platform-facade \
//!     --features spill-redb,durability-test-hooks \
//!     --test spill_redb_finalize_kill -- --nocapture
//! ```

#![cfg(feature = "durability-test-hooks")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

#[cfg(feature = "revocable_store")]
use quanta_memory_platform_core::{MemoryClass, MemoryPreset, StoreBindingHandle};
use quanta_memory_platform_facade::spill::backings::redb::RedbSpillBacking;
use quanta_memory_platform_facade::spill::service::{
    DurableRestoreTarget, ReconciledDurableSpillTarget,
};
use quanta_memory_platform_facade::spill::{DurableRecordId, DurableRecordResolution};
#[cfg(feature = "revocable_store")]
use quanta_memory_platform_facade::{
    DurableRestoreController, DurableRestoreOutcome, MemoryPlatformBuilder,
    ReconciledDurableSpillController, RevocableStore, RoutedSpillController,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique scratch database path removed on drop, even on assertion failure.
struct ScratchDb {
    path: PathBuf,
}

impl ScratchDb {
    fn new(tag: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("qmp-durability-l1-{}-{id}-{tag}.redb", std::process::id(),));
        Self { path }
    }
}

impl Drop for ScratchDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bounded repetition guard: fresh database per iteration, same oracles.
/// Unparseable or zero values fail closed to exactly one iteration.
fn iterations() -> usize {
    std::env::var("QMP_FINALIZE_KILL_ITERATIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(1)
}

/// Build and locate the finalize-kill child binary exactly once per test
/// process. A direct `cargo test --test ...` does not rebuild examples, so
/// merely accepting an existing path can execute a stale helper and invalidate
/// the kill boundary. The locked offline build is usually a no-op, but makes
/// source freshness an executable prerequisite.
fn finalize_kill_child_binary() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let direct = std::env::current_exe()
                .expect("current test binary")
                .parent()
                .expect("deps dir")
                .parent()
                .expect("target dir")
                .join("examples")
                .join(format!("spill_redb_finalize_kill_child{}", std::env::consts::EXE_SUFFIX));
            let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("crates dir")
                .parent()
                .expect("workspace root")
                .to_path_buf();
            let build = Command::new("cargo")
                .arg("build")
                .arg("--locked")
                .arg("--offline")
                .arg("-p")
                .arg("quanta-memory-platform-facade")
                .arg("--features")
                .arg("spill-redb,durability-test-hooks,revocable_store")
                .arg("--example")
                .arg("spill_redb_finalize_kill_child")
                .current_dir(&workspace_root)
                .output()
                .expect("freshness build for finalize-kill child");
            assert!(
                build.status.success(),
                "finalize-kill child freshness build failed: {}",
                String::from_utf8_lossy(&build.stderr)
            );
            assert!(direct.is_file(), "finalize-kill child binary missing at {}", direct.display());
            direct
        })
        .clone()
}

/// Parsed `BACKEND` + `ARMED` handshake from the blocked child.
struct Armed {
    backend: u128,
    point: String,
    sequence: u64,
    generation: u64,
    charged_bytes: Option<u64>,
}

/// Spawn the child, returning its handle plus the parsed handshake. `ARMED`
/// must arrive within the timeout; anything else (early exit, timeout,
/// unreadable pipe, malformed handshake) fails the test with the child
/// stderr attached.
fn spawn_until_armed(db: &Path, mode: &str) -> (Child, Armed) {
    let mut child = Command::new(finalize_kill_child_binary())
        .arg(db)
        .arg(mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn finalize-kill child");
    let stdout = child.stdout.take().expect("piped stdout");
    let (lines_tx, lines_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut backend = None;
    let mut charged_bytes = None;
    // The child performs durable writes before arming. Under a concurrent
    // workspace build, 30 seconds can expire before it reaches the hook;
    // retain a finite deadline without turning host load into a false crash.
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!remaining.is_zero(), "finalize-kill child never printed ARMED");
        match lines_rx.recv_timeout(remaining) {
            Ok(line) => {
                if let Some(id) = line.strip_prefix("BACKEND ") {
                    backend = Some(id.parse::<u128>().expect("backend identity parses"));
                } else if let Some(bytes) = line.strip_prefix("CHARGE ") {
                    charged_bytes = Some(bytes.parse::<u64>().expect("resident charge parses"));
                } else if let Some(rest) = line.strip_prefix("ARMED ") {
                    let mut parts = rest.split_whitespace();
                    let armed = Armed {
                        backend: backend.expect("BACKEND precedes ARMED"),
                        point: parts.next().expect("ARMED point").to_string(),
                        sequence: parts.next().expect("ARMED sequence").parse().expect("sequence"),
                        generation: parts
                            .next()
                            .expect("ARMED generation")
                            .parse()
                            .expect("generation"),
                        charged_bytes,
                    };
                    assert!(
                        parts.next().is_none(),
                        "ARMED handshake has exactly three fields, got {rest:?}"
                    );
                    return (child, armed);
                }
            }
            Err(_) => {
                let _ = child.kill();
                let output = child.wait_with_output().expect("reap silent child");
                panic!(
                    "finalize-kill child produced no ARMED (status {}):\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[cfg(feature = "revocable_store")]
#[test]
fn durability_l1_store_and_ledger_recover_across_finalize_kill() {
    for (mode, expected_point) in
        [("store-before", "FinalizeBeforeCommit"), ("store-after", "FinalizeAfterCommit")]
    {
        let db = ScratchDb::new(mode);
        let (mut child, armed) = spawn_until_armed(&db.path, mode);
        assert_eq!(armed.point, expected_point);
        assert_eq!(armed.sequence, 0);
        assert!(armed.charged_bytes.is_some_and(|bytes| bytes > 0));
        sigkill_child(&mut child);

        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("survivor platform");
        let store = platform
            .revocable_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ArtifactCache,
                "durability-store-survivor".to_string(),
                None,
            ))
            .build()
            .expect("survivor store");
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&db.path).expect("cold reopen"));
        let decode = |bytes: &[u8]| {
            assert_eq!(bytes.len(), 32, "whole payload on restart");
            let mut index = [0u8; 8];
            index.copy_from_slice(&bytes[..8]);
            let index = u64::from_le_bytes(index);
            assert_eq!(bytes, expected_payload(index));
            Ok((format!("k{index}"), bytes.to_vec()))
        };

        if mode == "store-before" {
            assert!(matches!(
                controller.resolve_record(DurableRecordId::new(0)).expect("record state"),
                DurableRecordResolution::Present { .. }
            ));
            assert!(matches!(
                store.recover_active_durable_restore(&controller, decode).expect("recover victim"),
                DurableRestoreOutcome::Restored { sequence: 0 }
            ));
            assert_eq!(
                store.reclaim_one().expect("read victim"),
                Some(("k0".to_string(), expected_payload(0)))
            );
        } else {
            assert_eq!(
                controller.resolve_record(DurableRecordId::new(0)).expect("record state"),
                DurableRecordResolution::Finalized,
                "terminal revocable handoff does not replay killed resident"
            );
            assert!(matches!(
                store
                    .recover_active_durable_restore(&controller, decode)
                    .expect("no victim replay"),
                DurableRestoreOutcome::Empty
            ));
        }
        assert!(matches!(
            store.restore_one_durable(&controller, decode).expect("restore unrelated"),
            DurableRestoreOutcome::Restored { sequence: 1 }
        ));
        assert_eq!(
            store.reclaim_one().expect("read unrelated"),
            Some(("k1".to_string(), expected_payload(1)))
        );
        assert!(matches!(
            store.restore_one_durable(&controller, decode).expect("second drain"),
            DurableRestoreOutcome::Empty
        ));
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(controller.active_restore(), Ok(None));
        assert_eq!(
            platform.broker().ledger_snapshot().expect("survivor ledger").total_owner_bytes(),
            0
        );
    }
}

/// End the child by force and prove it died by the kill — not by exiting on
/// its own (which would downgrade this rail to a clean-shutdown test wearing
/// a crash costume).
fn sigkill_child(child: &mut Child) {
    if let Err(error) = child.kill() {
        // The child died before the kill (a real crash, not a clean exit —
        // no mode exits on its own after ARMED): reap it and show why instead
        // of a bare kill error.
        let mut stderr = Vec::new();
        if let Some(mut pipe) = child.stderr.take() {
            use std::io::Read;
            let _ = pipe.read_to_end(&mut stderr);
        }
        let status = child.wait().expect("reap dead child");
        panic!(
            "finalize-kill child died before SIGKILL (kill error: {error}, status {status}):\n{}",
            String::from_utf8_lossy(&stderr)
        );
    }
    let status = child.wait().expect("reap the finalize-kill child");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9), "child must die by SIGKILL, got {status:?}");
    }
    #[cfg(not(unix))]
    {
        assert!(!status.success(), "child must not exit cleanly, got {status:?}");
    }
}

/// 32-byte self-describing payload the child commits: index plus filler.
/// Asserting full byte equality pins sequence and checksum at once.
fn expected_payload(index: u64) -> Vec<u8> {
    let mut bytes = vec![0xA5u8; 32];
    bytes[..8].copy_from_slice(&index.to_le_bytes());
    bytes
}

/// Drain every record oldest-first through the full
/// prepare -> commit -> finalize protocol, returning `(sequence, bytes)`
/// pairs. Any ambiguity fails the test: after a kill, recovery must converge,
/// not strand records behind unknown outcomes.
fn drain_all(backing: &RedbSpillBacking) -> Vec<(u64, Vec<u8>)> {
    let mut drained = Vec::new();
    while let Some(candidate) = backing.prepare_restore().expect("prepare") {
        let sequence = candidate.sequence;
        let bytes = candidate.bytes().to_vec();
        backing.commit_restore(&candidate.claim).expect("commit converges after kill");
        backing.finalize_restore(&candidate.claim).expect("finalize");
        assert_eq!(bytes.len(), 32, "no torn payload survives a kill");
        let mut index = [0u8; 8];
        index.copy_from_slice(&bytes[..8]);
        assert_eq!(
            bytes,
            expected_payload(u64::from_le_bytes(index)),
            "payload sequence and checksum survive the kill on sequence {sequence}"
        );
        drained.push((sequence, bytes));
    }
    drained
}

#[test]
fn durability_l1_finalize_before_commit_kill_recovers_present_and_drains_exactly_once() {
    for _ in 0..iterations() {
        let db = ScratchDb::new("before");
        let (mut handle, armed) = spawn_until_armed(&db.path, "before");
        assert_eq!(armed.point, "FinalizeBeforeCommit", "armed at the staged point");
        assert_eq!(armed.sequence, 0, "the victim is the oldest record");
        assert_eq!(armed.charged_bytes, None, "backing-only mode has no broker charge");
        sigkill_child(&mut handle);

        // The survivor reopens the same file cold: the uncommitted finalize
        // left the record Present, and recovery mints a FRESH generation
        // bound to the SAME persisted backend.
        let backing = RedbSpillBacking::create(&db.path).expect("cold reopen");
        assert!(
            matches!(
                backing.resolve_record(DurableRecordId::new(0)).expect("resolve"),
                DurableRecordResolution::Present { .. }
            ),
            "a kill before commit leaves the record Present, never Finalized or Absent"
        );
        let recovery = backing
            .recover_active_restore()
            .expect("recover")
            .expect("interrupted finalize survives SIGKILL");
        assert_eq!(
            recovery.candidate.claim.backend(),
            armed.backend,
            "backend identity is stable across SIGKILL, not re-minted"
        );
        assert_ne!(
            recovery.candidate.claim.generation(),
            armed.generation,
            "recovery issues a fresh exclusive generation"
        );
        assert_eq!(recovery.candidate.sequence, 0, "oldest record restores first");
        backing.commit_restore(&recovery.candidate.claim).expect("recovered claim commits");
        backing.finalize_restore(&recovery.candidate.claim).expect("recovered claim finalizes");

        let mut drained = vec![(0u64, expected_payload(0))];
        drained.extend(drain_all(&backing));
        assert_eq!(drained.len(), 2, "victim plus unrelated record drain exactly once");
        for (position, (sequence, bytes)) in drained.iter().enumerate() {
            assert_eq!(*sequence, position as u64, "oldest-first order survives the kill");
            assert_eq!(*bytes, expected_payload(position as u64), "payload survives the kill");
        }
        assert!(drain_all(&backing).is_empty(), "a second drain is empty");
        assert_eq!(backing.retained_count().expect("count"), 0, "exact drain");
        assert_eq!(backing.active_restore().expect("active"), None, "no stranded claim");
    }
}

#[test]
fn durability_l1_finalize_after_commit_kill_resolves_finalized_without_resurrection() {
    for _ in 0..iterations() {
        let db = ScratchDb::new("after");
        let (mut handle, armed) = spawn_until_armed(&db.path, "after");
        assert_eq!(armed.point, "FinalizeAfterCommit", "armed at the committed point");
        assert_eq!(armed.sequence, 0, "the victim is the oldest record");
        assert_eq!(armed.charged_bytes, None, "backing-only mode has no broker charge");
        sigkill_child(&mut handle);

        // The survivor reopens the same file cold: the committed finalize is
        // exactly Finalized with no recovery payload and no live claim — and
        // the victim can never be offered or recreated again.
        let backing = RedbSpillBacking::create(&db.path).expect("cold reopen");
        assert_eq!(
            backing.resolve_record(DurableRecordId::new(0)).expect("resolve"),
            DurableRecordResolution::Finalized,
            "a kill after commit leaves the exact record Finalized"
        );
        assert!(
            backing.recover_active_restore().expect("recover").is_none(),
            "no recovery payload survives a committed finalize"
        );
        assert_eq!(
            backing.active_restore().expect("active"),
            None,
            "no active claim survives a committed finalize"
        );
        assert!(
            matches!(
                backing.resolve_record(DurableRecordId::new(1)).expect("resolve"),
                DurableRecordResolution::Present { .. }
            ),
            "the unrelated record is untouched by the victim's finalize"
        );
        let drained = drain_all(&backing);
        assert_eq!(drained.len(), 1, "only the unrelated record drains");
        assert_eq!(drained[0].0, 1, "FIFO prepare skips the finalized victim");
        assert_eq!(drained[0].1, expected_payload(1), "unrelated payload is intact");
        assert!(
            backing.prepare_restore().expect("prepare").is_none(),
            "ordinary FIFO prepare is empty once the victim is finalized"
        );
        let mut different = expected_payload(0);
        different[0] ^= 0xFF;
        assert!(
            backing.commit_spill(DurableRecordId::new(0), &expected_payload(0)).is_err(),
            "a finalized id cannot be recreated with identical bytes"
        );
        assert!(
            backing.commit_spill(DurableRecordId::new(0), &different).is_err(),
            "a finalized id cannot be recreated with different bytes"
        );
        assert_eq!(backing.retained_count().expect("count"), 0, "exact drain");
    }
}
